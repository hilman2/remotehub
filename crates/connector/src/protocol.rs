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
    /// Say whether the customer's access lets a connection to `target`
    /// through now (#180), without connecting. The connector answers with
    /// [`Report::Checked`]; only it can resolve the customer's host names.
    Check { id: Uuid, target: String },
}

/// What a connector sends on the control socket.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Report {
    /// The stream `id` could not be opened; `not_open` if the customer has
    /// not opened the target (#180).
    Failed {
        id: Uuid,
        reason: String,
        #[serde(default)]
        not_open: bool,
    },
    /// The answer to [`Control::Check`] `id`: whether the target is open,
    /// and until when (RFC 3339), the latest end of what opens it; none while
    /// closed or open without end.
    Checked {
        id: Uuid,
        open: bool,
        #[serde(default)]
        until: Option<String>,
    },
}

/// Where a connector reports whether its customer lets remotehub in (#165),
/// with a `POST` of [`State`]. It does so every [`STATE_EVERY`] and on each
/// change, whether open or closed: a closed connector has no control socket.
/// remotehub answers with [`Pending`], the requests for access waiting for
/// the customer (#181): data the connector shows, never a command.
pub const STATE_PATH: &str = "/api/connectors/state";

/// Often enough that a request for access reaches the customer while the
/// technician waits.
pub const STATE_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// The most remotehub may answer a report with, in bytes; the connector drops
/// a longer answer whole.
pub const MAX_PENDING_BYTES: usize = 64 * 1024;
/// The most requests an answer may hold, and targets a request.
pub const MAX_REQUESTS: usize = 20;
pub const MAX_TARGETS: usize = 20;
/// The longest reason and the longest name, in characters.
pub const MAX_REASON: usize = 500;
pub const MAX_NAME: usize = 200;
/// How long access may be asked for, in minutes: a quarter of an hour to a
/// day.
pub const MINUTES: std::ops::RangeInclusive<u32> = 15..=1440;

/// The requests for access waiting for the customer, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub requests: Vec<CustomerRequest>,
}

/// A remotehub user asks the customer to open these targets for a while.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomerRequest {
    pub id: Uuid,
    /// The remotehub user who asks, as remotehub names them.
    pub requester: String,
    pub reason: String,
    pub minutes: u32,
    pub targets: Vec<RequestTarget>,
}

/// A device remotehub would connect to: its name in remotehub, and the
/// address and port an approval opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTarget {
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// The customer's answer to a request, sent with every report until
/// remotehub no longer lists the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub id: Uuid,
    pub approved: bool,
    /// The connector user who answered.
    pub by: String,
    /// RFC 3339, the end of an approval.
    pub until: Option<String>,
}

/// A report names at most this many groups and as many devices; remotehub
/// refuses a longer one.
pub const MAX_LISTED: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Whether the whole network is open.
    pub open: bool,
    /// RFC 3339, the end of the whole network's access; none while closed or
    /// open without end.
    pub until: Option<String>,
    /// Whether single devices or groups are open while the whole network is
    /// not (#180).
    #[serde(default)]
    pub partly: bool,
    /// The open groups of the customer's list.
    #[serde(default)]
    pub groups: Vec<OpenGroup>,
    /// The open devices of the customer's list, by themselves or through a
    /// group, and the targets of approved requests (#181).
    #[serde(default)]
    pub devices: Vec<OpenDevice>,
    /// The customer's answers to requests remotehub still lists (#181).
    #[serde(default)]
    pub answers: Vec<Answer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenGroup {
    pub name: String,
    /// RFC 3339; none while open without end.
    pub until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDevice {
    pub name: String,
    /// As the customer wrote it: an address, a range or a host name.
    pub address: String,
    /// Such as `22,8000-8100`.
    pub ports: String,
    /// RFC 3339, the latest end of what opens it; none while open without
    /// end.
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
                reason: "refused".into(),
                not_open: false,
            }
        );
    }
}
