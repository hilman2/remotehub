//! Whether the customer lets remotehub into this network (#165): closed,
//! open until a point in time, or open without end.
//!
//! The state lives in `access.json` in the data directory. The web interface
//! and the command line both change it through [`change`], which also writes
//! the journal; the running connector follows the file through a [`Gate`],
//! so a change from the command line reaches it too.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::watch;

use crate::journal::{Event, Journal};

/// How often the connector reads the file for changes from the command line
/// and checks whether the time ran out.
const FOLLOW: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Access {
    /// remotehub cannot reach the network: the default.
    #[default]
    Closed,
    /// Open until `until`, or without end.
    Open {
        #[serde(with = "time::serde::rfc3339::option")]
        until: Option<OffsetDateTime>,
    },
}

impl Access {
    /// Whether remotehub may reach the network at `now`.
    pub fn is_open(&self, now: OffsetDateTime) -> bool {
        match self {
            Access::Closed => false,
            Access::Open { until: None } => true,
            Access::Open { until: Some(until) } => *until > now,
        }
    }

    /// When it closes by itself, if it is open until a point in time.
    pub fn closes_at(&self) -> Option<OffsetDateTime> {
        match self {
            Access::Open { until } => *until,
            Access::Closed => None,
        }
    }
}

/// Who changed the access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "via", rename_all = "snake_case")]
pub enum Changer {
    /// A user of the web interface.
    Web {
        user: String,
    },
    CommandLine,
    /// The connector itself, when the time ran out.
    Expiry,
}

/// What `access.json` holds: the access and its last change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Stored {
    pub access: Access,
    pub changed_by: Option<Changer>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub changed_at: Option<OffsetDateTime>,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("access.json")
}

/// The stored state; closed if nothing was stored yet.
pub fn load(dir: &Path) -> io::Result<Stored> {
    match std::fs::read(path(dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(error) => Err(error),
    }
}

/// Stores `access` as changed by `by` now, and writes the change to the
/// journal. Times keep whole seconds: nobody opens access to the
/// nanosecond, and the log reads better without.
pub fn change(dir: &Path, journal: &Journal, access: Access, by: Changer) -> io::Result<Stored> {
    let seconds = |at: OffsetDateTime| at.replace_nanosecond(0).unwrap_or(at);
    let access = match access {
        Access::Open { until } => Access::Open {
            until: until.map(seconds),
        },
        Access::Closed => Access::Closed,
    };
    let stored = Stored {
        access,
        changed_by: Some(by.clone()),
        changed_at: Some(seconds(OffsetDateTime::now_utc())),
    };
    // Written beside it and renamed, so a reader never sees half a file.
    let temporary = dir.join("access.json.new");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&stored)?)?;
    std::fs::rename(&temporary, path(dir))?;
    journal.append(match access {
        Access::Closed if by == Changer::Expiry => Event::Expired,
        Access::Closed => Event::Closed { by },
        Access::Open { until } => Event::Opened { by, until },
    });
    Ok(stored)
}

/// The access as the running connector sees it.
pub struct Gate {
    dir: PathBuf,
    journal: Arc<Journal>,
    state: watch::Sender<Stored>,
}

impl Gate {
    pub fn new(dir: &Path, journal: Arc<Journal>) -> io::Result<Arc<Gate>> {
        let (state, _) = watch::channel(load(dir)?);
        Ok(Arc::new(Gate {
            dir: dir.to_owned(),
            journal,
            state,
        }))
    }

    pub fn current(&self) -> Stored {
        self.state.borrow().clone()
    }

    /// Sees every change from now on.
    pub fn subscribe(&self) -> watch::Receiver<Stored> {
        self.state.subscribe()
    }

    /// Changes the access, as the web interface does.
    pub fn set(&self, access: Access, by: Changer) -> io::Result<()> {
        let stored = change(&self.dir, &self.journal, access, by)?;
        self.state.send_replace(stored);
        Ok(())
    }

    /// Runs until the process ends: takes over changes from the command line,
    /// and closes the access when its time runs out.
    pub async fn follow(self: Arc<Self>) {
        let mut tick = tokio::time::interval(FOLLOW);
        loop {
            tick.tick().await;
            match load(&self.dir) {
                Ok(stored) => {
                    let taken = self.state.send_if_modified(|current| {
                        let changed = *current != stored;
                        *current = stored.clone();
                        changed
                    });
                    // The command line's own process wrote the journal; the
                    // log output of the running connector says it, too.
                    if taken {
                        tracing::info!(access = ?stored.access, by = ?stored.changed_by, "access changed");
                    }
                }
                Err(error) => tracing::warn!(%error, "cannot read access.json"),
            }
            let current = self.current().access;
            let ran_out =
                current.closes_at().is_some() && !current.is_open(OffsetDateTime::now_utc());
            if ran_out && let Err(error) = self.set(Access::Closed, Changer::Expiry) {
                tracing::warn!(%error, "cannot close the access");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn open_means_before_its_end() {
        let now = datetime!(2026-10-01 12:00 UTC);
        assert!(!Access::Closed.is_open(now));
        assert!(Access::Open { until: None }.is_open(now));
        let until = Access::Open {
            until: Some(datetime!(2026-10-01 13:00 UTC)),
        };
        assert!(until.is_open(now));
        assert!(!until.is_open(datetime!(2026-10-01 13:00 UTC)));
    }

    #[test]
    fn nothing_stored_is_closed_and_a_change_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::new(dir.path());
        assert_eq!(load(dir.path()).unwrap().access, Access::Closed);
        let until = Some(datetime!(2026-10-01 13:00 UTC));
        change(
            dir.path(),
            &journal,
            Access::Open { until },
            Changer::CommandLine,
        )
        .unwrap();
        let stored = load(dir.path()).unwrap();
        assert_eq!(stored.access, Access::Open { until });
        assert_eq!(stored.changed_by, Some(Changer::CommandLine));
        let logged = journal.recent(10);
        assert_eq!(
            logged[0].event,
            Event::Opened {
                by: Changer::CommandLine,
                until
            }
        );
    }

    /// The running connector takes over a change the command line wrote, and
    /// closes the access itself once its time ran out.
    #[tokio::test]
    async fn the_gate_follows_the_file_and_the_clock() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::new(dir.path()));
        let gate = Gate::new(dir.path(), journal.clone()).unwrap();
        let mut seen = gate.subscribe();
        let _follow = tokio::spawn(gate.clone().follow());

        let until = OffsetDateTime::now_utc() + Duration::from_secs(2);
        change(
            dir.path(),
            &journal,
            Access::Open { until: Some(until) },
            Changer::CommandLine,
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), seen.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(
            seen.borrow_and_update()
                .access
                .is_open(OffsetDateTime::now_utc())
        );

        tokio::time::timeout(Duration::from_secs(5), seen.changed())
            .await
            .unwrap()
            .unwrap();
        let closed = seen.borrow().clone();
        assert_eq!(closed.access, Access::Closed);
        assert_eq!(closed.changed_by, Some(Changer::Expiry));
        assert_eq!(journal.recent(1)[0].event, Event::Expired);
    }
}
