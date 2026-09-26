//! The customer's own record of remote access (#165): who opened and closed
//! it, and every connection remotehub made through the connector.
//!
//! One JSON object per line in `access.log` in the data directory, and the
//! same on the log output (stdout, `docker logs`) and, on Windows, in the
//! event log. The web interface shows the latest entries.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::access::Changer;

/// `access.log` moves to `access.log.1` beyond this size, replacing the
/// previous one.
const ROTATE_AT: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Opened {
        by: Changer,
        #[serde(with = "time::serde::rfc3339::option")]
        until: Option<OffsetDateTime>,
    },
    Closed {
        by: Changer,
    },
    /// The access closed when its time ran out.
    Expired,
    /// `device` is the device's name in remotehub and `user` the remotehub
    /// user, both as remotehub reports them; the connector cannot check
    /// them. `target` is what it connected to.
    ConnectionStarted {
        id: Uuid,
        target: String,
        #[serde(default)]
        device: Option<String>,
        user: Option<String>,
    },
    /// `sent` went to the device, `received` came from it, in bytes.
    ConnectionEnded {
        id: Uuid,
        target: String,
        #[serde(default)]
        device: Option<String>,
        user: Option<String>,
        seconds: u64,
        sent: u64,
        received: u64,
    },
    ConnectionRefused {
        target: String,
        #[serde(default)]
        device: Option<String>,
        user: Option<String>,
        reason: String,
    },
    SignedIn {
        user: String,
        address: String,
    },
    SignInFailed {
        user: String,
        address: String,
    },
    /// Too many failed sign-ins: the user is locked for a while.
    Locked {
        user: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    #[serde(flatten)]
    pub event: Event,
}

pub struct Journal {
    path: PathBuf,
    /// Keeps the lines of this process whole; the command line appends from
    /// a process of its own, a line at a time.
    writing: Mutex<()>,
}

impl Journal {
    pub fn new(dir: &Path) -> Journal {
        Journal {
            path: dir.join("access.log"),
            writing: Mutex::new(()),
        }
    }

    /// Records `event` now. A journal that cannot be written is reported on
    /// the log output, which gets every entry anyway.
    pub fn append(&self, event: Event) {
        let entry = Entry {
            at: OffsetDateTime::now_utc(),
            event,
        };
        let line = serde_json::to_string(&entry).expect("entries serialize");
        tracing::info!(target: "journal", "{line}");
        #[cfg(windows)]
        crate::windows::eventlog::journal(&entry.event, &line);
        let _writing = self.writing.lock().expect("no panics while locked");
        if let Err(error) = self.write(&line) {
            tracing::error!(%error, path = %self.path.display(), "cannot write the journal");
        }
    }

    fn write(&self, line: &str) -> io::Result<()> {
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() > ROTATE_AT) {
            std::fs::rename(&self.path, self.path.with_extension("log.1"))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        // One write per line: appends of other processes land between
        // lines, not inside one.
        file.write_all(format!("{line}\n").as_bytes())
    }

    /// The latest `count` entries, newest first. Lines that do not parse,
    /// e.g. from a newer release, are left out.
    pub fn recent(&self, count: usize) -> Vec<Entry> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        text.lines()
            .rev()
            .filter_map(|line| serde_json::from_str(line).ok())
            .take(count)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_come_back_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::new(dir.path());
        assert!(journal.recent(5).is_empty());
        journal.append(Event::Locked { user: "a".into() });
        journal.append(Event::Expired);
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join("access.log"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();
        let events: Vec<Event> = journal.recent(5).into_iter().map(|e| e.event).collect();
        assert_eq!(events, [Event::Expired, Event::Locked { user: "a".into() }]);
        assert_eq!(journal.recent(1).len(), 1);
    }
}
