//! Requests for access from remotehub (#181, ADR 0013).
//!
//! remotehub answers each state report with the requests waiting for the
//! customer. The answer comes from outside the customer's network, so it is
//! data and nothing else: [`parse`] takes it only whole and strictly bounded,
//! and nothing here changes the access. A signed-in user of the web interface
//! approves or refuses a request; the answer goes back with the next report.

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Mutex;

use tokio::sync::Notify;
use uuid::Uuid;

use crate::protocol::{
    Answer, CustomerRequest, MAX_NAME, MAX_PENDING_BYTES, MAX_REASON, MAX_REQUESTS, MAX_TARGETS,
    MINUTES, Pending,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RequestsError {
    #[error("longer than {MAX_PENDING_BYTES} bytes")]
    TooLong,
    #[error("not the expected JSON: {0}")]
    Malformed(String),
    #[error("more than {MAX_REQUESTS} requests")]
    TooMany,
    #[error("request {0} twice")]
    Twice(Uuid),
    #[error("request {0}: {1}")]
    Invalid(Uuid, &'static str),
}

/// remotehub's answer to a report, if every part of it is as expected;
/// otherwise none of it.
pub fn parse(bytes: &[u8]) -> Result<Vec<CustomerRequest>, RequestsError> {
    if bytes.len() > MAX_PENDING_BYTES {
        return Err(RequestsError::TooLong);
    }
    let pending: Pending =
        serde_json::from_slice(bytes).map_err(|e| RequestsError::Malformed(e.to_string()))?;
    if pending.requests.len() > MAX_REQUESTS {
        return Err(RequestsError::TooMany);
    }
    let mut seen = HashSet::new();
    for request in &pending.requests {
        let invalid = |what| Err(RequestsError::Invalid(request.id, what));
        if !seen.insert(request.id) {
            return Err(RequestsError::Twice(request.id));
        }
        if !plain(&request.requester, MAX_NAME) {
            return invalid("requester");
        }
        if !plain(&request.reason, MAX_REASON) {
            return invalid("reason");
        }
        if !MINUTES.contains(&request.minutes) {
            return invalid("minutes");
        }
        if request.targets.is_empty() || request.targets.len() > MAX_TARGETS {
            return invalid("targets");
        }
        for target in &request.targets {
            if !plain(&target.name, MAX_NAME) {
                return invalid("target name");
            }
            if !host(&target.host) {
                return invalid("target host");
            }
            if target.port == 0 {
                return invalid("target port");
            }
        }
    }
    Ok(pending.requests)
}

/// Whether `text` is 1 to `longest` characters that read as what they are:
/// no control characters, and none that reorder or hide text, which could
/// make a name look like another.
pub fn plain(text: &str, longest: usize) -> bool {
    let count = text.chars().count();
    (1..=longest).contains(&count) && !text.chars().any(|c| c.is_control() || hidden(c))
}

/// `text` made [`plain`]: each character that is not becomes U+FFFD, and it
/// is cut to `longest` characters. remotehub writes names this way, so a
/// device's odd name never makes the connector drop a whole answer.
pub fn readable(text: &str, longest: usize) -> String {
    let text: String = text
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || hidden(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .take(longest)
        .collect();
    if text.is_empty() {
        "\u{FFFD}".to_owned()
    } else {
        text
    }
}

/// Characters that change the direction of text or take no space.
fn hidden(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

/// An IP address, or a host name of letters, digits and hyphens.
pub fn host(text: &str) -> bool {
    if text.parse::<IpAddr>().is_ok() {
        return true;
    }
    (1..=253).contains(&text.len())
        && text.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

/// The requests remotehub listed last, and the customer's answers to them.
#[derive(Default)]
pub struct Requests {
    state: Mutex<State>,
    /// Wakes the report, so an answer reaches remotehub at once.
    pub answered: Notify,
}

#[derive(Default)]
struct State {
    listed: Vec<CustomerRequest>,
    answers: Vec<Answer>,
}

impl Requests {
    /// Takes remotehub's latest list; returns the requests it lists for the
    /// first time. Answers to requests it no longer lists have arrived.
    pub fn listed(&self, requests: Vec<CustomerRequest>) -> Vec<CustomerRequest> {
        let mut state = self.state.lock().expect("no panics while locked");
        let known: HashSet<Uuid> = state.listed.iter().map(|r| r.id).collect();
        let new = requests
            .iter()
            .filter(|r| !known.contains(&r.id))
            .cloned()
            .collect();
        let ids: HashSet<Uuid> = requests.iter().map(|r| r.id).collect();
        state.answers.retain(|answer| ids.contains(&answer.id));
        state.listed = requests;
        new
    }

    /// The requests the customer has not answered, oldest first.
    pub fn waiting(&self) -> Vec<CustomerRequest> {
        let state = self.state.lock().expect("no panics while locked");
        state
            .listed
            .iter()
            .filter(|r| !state.answers.iter().any(|a| a.id == r.id))
            .cloned()
            .collect()
    }

    /// The request `id`, if it waits for an answer.
    pub fn get(&self, id: Uuid) -> Option<CustomerRequest> {
        self.waiting().into_iter().find(|r| r.id == id)
    }

    /// The answers remotehub has not taken yet.
    pub fn answers(&self) -> Vec<Answer> {
        self.state
            .lock()
            .expect("no panics while locked")
            .answers
            .clone()
    }

    /// Records the customer's answer and wakes the report.
    pub fn answer(&self, answer: Answer) {
        self.state
            .lock()
            .expect("no panics while locked")
            .answers
            .push(answer);
        self.answered.notify_one();
    }
}

#[cfg(test)]
mod tests;
