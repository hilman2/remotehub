//! Signs in on the device's page: waits for its sign-in form, types user
//! name and password as keystrokes would, and presses Enter. Typing through
//! `Input.insertText` fires the input events that script-driven forms
//! listen to; setting `value` would not.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use crate::cdp::{Cdp, CdpError};

const SCRIPT: &str = include_str!("fill.js");
/// How often to look for the form while the page loads.
const POLL: Duration = Duration::from_millis(250);

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Filled,
    /// No sign-in form showed up in time.
    NoForm,
    /// Chromium showed its own error page: the device did not answer, or
    /// presented a certificate other than the pinned one.
    PageError,
}

/// Fills in the form of the page at `origin` once it shows one, waiting at
/// most `patience`. An empty `username` types only the password.
pub async fn fill(
    cdp: &mut Cdp,
    origin: &str,
    username: &str,
    password: &SecretString,
    patience: Duration,
) -> Result<Outcome, CdpError> {
    let page = page_session(cdp).await?;
    let page = Some(page.as_str());
    let deadline = tokio::time::Instant::now() + patience;
    let form = loop {
        match find(cdp, page, origin, None).await? {
            Some(form) if form["error"] == true => return Ok(Outcome::PageError),
            Some(form) => break form,
            None => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(Outcome::NoForm);
        }
        tokio::time::sleep(POLL).await;
    };
    // Each field is found and focused again right before typing into it: the
    // page may have changed since, and keystrokes go wherever the focus is.
    let focused = |found: Option<Value>| found.is_some_and(|form| form["error"] != true);
    if !username.is_empty()
        && form["user"] == true
        && focused(find(cdp, page, origin, Some("user")).await?)
    {
        cdp.call(page, "Input.insertText", json!({ "text": username }))
            .await?;
    }
    if !focused(find(cdp, page, origin, Some("password")).await?) {
        return Ok(Outcome::NoForm);
    }
    cdp.call(
        page,
        "Input.insertText",
        json!({ "text": password.expose_secret() }),
    )
    .await?;
    for (kind, text) in [("keyDown", "\r"), ("keyUp", "")] {
        cdp.call(
            page,
            "Input.dispatchKeyEvent",
            json!({ "type": kind, "key": "Enter", "code": "Enter",
                    "windowsVirtualKeyCode": 13, "text": text }),
        )
        .await?;
    }
    Ok(Outcome::Filled)
}

/// Attaches to the browser's first page (kiosk mode opens exactly one).
async fn page_session(cdp: &mut Cdp) -> Result<String, CdpError> {
    let targets = cdp.call(None, "Target.getTargets", json!({})).await?;
    let page = targets["targetInfos"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|target| target["type"] == "page")
        .and_then(|target| target["targetId"].as_str())
        .ok_or_else(|| CdpError::Remote {
            method: "Target.getTargets".into(),
            message: "no page".into(),
        })?
        .to_owned();
    let attached = cdp
        .call(
            None,
            "Target.attachToTarget",
            json!({ "targetId": page, "flatten": true }),
        )
        .await?;
    attached["sessionId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| CdpError::Remote {
            method: "Target.attachToTarget".into(),
            message: "no session".into(),
        })
}

/// Runs [`SCRIPT`] in the page; see there for what `which` does.
async fn find(
    cdp: &mut Cdp,
    page: Option<&str>,
    origin: &str,
    which: Option<&str>,
) -> Result<Option<Value>, CdpError> {
    let expression = format!("({SCRIPT})({}, {})", json!(origin), json!(which));
    let result = match cdp
        .call(
            page,
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )
        .await
    {
        Ok(result) => result,
        // A page that is navigating has no context to run in yet.
        Err(CdpError::Remote { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(result["result"]["value"]
        .as_object()
        .map(|form| Value::Object(form.clone())))
}
