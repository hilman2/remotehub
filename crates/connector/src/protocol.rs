//! What a connector and remotehub say to each other on the control socket.
//! Both sides use these types, so a change here changes both at once; the
//! protocol version tells a connector and a remotehub of different releases
//! apart.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The header each request of a connector carries its protocol version in.
pub const HEADER: &str = "remotehub-connector-protocol";

/// Raise it with every change a connector of the previous version would
/// misread. remotehub refuses connectors of any other version, and names its
/// own in the same header of the refusal.
pub const VERSION: u32 = 1;

/// What remotehub sends on the control socket.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// Connect to `target` (`host:port`) and open the stream `id`, for the
    /// remotehub user `user` and the device named `device` in remotehub; the
    /// connector names both in its journal (#177).
    Open {
        id: Uuid,
        target: String,
        user: Option<String>,
        #[serde(default)]
        device: Option<String>,
    },
}

/// What a connector sends on the control socket.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Report {
    /// The stream `id` could not be opened.
    Failed { id: Uuid, reason: String },
}

/// Where a connector reports whether its customer lets remotehub in (#165),
/// with a `POST` of [`State`]. It does so every [`STATE_EVERY`] and on each
/// change, whether open or closed: a closed connector has no control socket.
pub const STATE_PATH: &str = "/api/connectors/state";

pub const STATE_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub open: bool,
    /// RFC 3339; none while closed or open without end.
    pub until: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_messages_are_json_the_other_side_reads() {
        let id = Uuid::nil();
        let open = serde_json::to_value(Control::Open {
            id,
            target: "ssh-target:22".into(),
            user: Some("alice".into()),
            device: Some("router".into()),
        })
        .unwrap();
        assert_eq!(
            open,
            serde_json::json!({ "type": "open", "id": id, "target": "ssh-target:22",
                                "user": "alice", "device": "router" })
        );
        let failed: Report = serde_json::from_str(&format!(
            r#"{{"type":"failed","id":"{id}","reason":"refused"}}"#
        ))
        .unwrap();
        assert_eq!(
            failed,
            Report::Failed {
                id,
                reason: "refused".into()
            }
        );
    }
}
