//! The Chrome DevTools Protocol over Chromium's `--remote-debugging-pipe`:
//! JSON messages separated by a NUL byte, requests on Chromium's file
//! descriptor 3, answers and events on 4. [`crate::session`] maps those to
//! the child's stdin and stdout. No TCP port, so nothing else in the
//! container can drive the browser.

use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

#[derive(Debug, Error)]
pub enum CdpError {
    #[error("the browser closed the pipe")]
    Closed,
    #[error("pipe: {0}")]
    Io(#[from] std::io::Error),
    #[error("unreadable message: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("{method}: {message}")]
    Remote { method: String, message: String },
}

pub struct Cdp {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next: u64,
}

impl Cdp {
    pub fn new(input: ChildStdin, output: ChildStdout) -> Self {
        Self {
            input,
            output: BufReader::new(output),
            next: 0,
        }
    }

    /// Sends `method` to the browser, or to a page if `session` names one,
    /// and returns the result. Events that arrive meanwhile are dropped:
    /// nothing here enables a domain that sends events worth reading.
    pub async fn call(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, CdpError> {
        self.next += 1;
        let id = self.next;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let mut bytes = serde_json::to_vec(&message)?;
        bytes.push(0);
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        loop {
            let mut raw = Vec::new();
            if self.output.read_until(0, &mut raw).await? == 0 {
                return Err(CdpError::Closed);
            }
            raw.pop_if(|byte| *byte == 0);
            let mut answer: Value = serde_json::from_slice(&raw)?;
            if answer["id"] != id {
                continue;
            }
            if let Some(error) = answer.get("error") {
                return Err(CdpError::Remote {
                    method: method.to_owned(),
                    message: error["message"].as_str().unwrap_or_default().to_owned(),
                });
            }
            return Ok(answer["result"].take());
        }
    }

    /// Keeps reading (and dropping) what the browser sends, so it never
    /// blocks on a full pipe, and returns the input end: Chromium ends when
    /// the pipe closes, so the caller keeps it for the session.
    pub fn idle(self) -> ChildStdin {
        let mut output = self.output;
        tokio::spawn(async move {
            let mut raw = Vec::new();
            while output
                .read_until(0, &mut raw)
                .await
                .is_ok_and(|read| read > 0)
            {
                raw.clear();
            }
        });
        self.input
    }
}
