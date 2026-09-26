//! Whether the customer lets remotehub into this network (#165): the whole
//! network, or single devices and groups of the customer's own list (#180),
//! each closed, open until a point in time, or open without end.
//!
//! The state lives in `access.json`, the list in `inventory.json`, both in
//! the data directory. The web interface and the command line change them
//! through the functions here, which also write the journal; the running
//! connector follows the files through a [`Gate`], so a change from the
//! command line reaches it too.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::watch;

use crate::inventory::{self, Device, Inventory, InventoryError};
use crate::journal::{Event, Journal};

/// How often the connector reads the files for changes from the command line
/// and checks whether a time ran out.
const FOLLOW: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Access {
    /// remotehub cannot reach it: the default.
    #[default]
    Closed,
    /// Open until `until`, or without end.
    Open {
        #[serde(with = "time::serde::rfc3339::option")]
        until: Option<OffsetDateTime>,
    },
}

impl Access {
    /// Whether remotehub may reach it at `now`.
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

/// What can be opened on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Every address `REMOTEHUB_CONNECTOR_ALLOW` covers, on every port.
    #[default]
    Network,
    Device {
        name: String,
    },
    Group {
        name: String,
    },
}

/// The access of one scope and its last change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Stored {
    pub access: Access,
    pub changed_by: Option<Changer>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub changed_at: Option<OffsetDateTime>,
}

/// What `access.json` holds; what it does not name is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccessFile {
    #[serde(default)]
    pub network: Stored,
    #[serde(default)]
    pub devices: BTreeMap<String, Stored>,
    #[serde(default)]
    pub groups: BTreeMap<String, Stored>,
}

impl AccessFile {
    pub fn get(&self, scope: &Scope) -> Stored {
        match scope {
            Scope::Network => Some(&self.network),
            Scope::Device { name } => self.devices.get(name),
            Scope::Group { name } => self.groups.get(name),
        }
        .cloned()
        .unwrap_or_default()
    }

    fn set(&mut self, scope: &Scope, stored: Stored) {
        match scope {
            Scope::Network => self.network = stored,
            Scope::Device { name } => {
                self.devices.insert(name.clone(), stored);
            }
            Scope::Group { name } => {
                self.groups.insert(name.clone(), stored);
            }
        }
    }

    /// Every scope with its access.
    fn all(&self) -> impl Iterator<Item = (Scope, &Stored)> {
        std::iter::once((Scope::Network, &self.network))
            .chain(
                self.devices
                    .iter()
                    .map(|(name, s)| (Scope::Device { name: name.clone() }, s)),
            )
            .chain(
                self.groups
                    .iter()
                    .map(|(name, s)| (Scope::Group { name: name.clone() }, s)),
            )
    }
}

/// Everything the connector decides by: the access and the list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub access: AccessFile,
    pub inventory: Inventory,
}

impl Snapshot {
    pub fn network_open(&self, now: OffsetDateTime) -> bool {
        self.access.network.access.is_open(now)
    }

    /// Until when the whole network is open at `now`: `Some(None)` without
    /// end, none while closed.
    pub fn network_until(&self, now: OffsetDateTime) -> Option<Option<OffsetDateTime>> {
        let access = self.access.network.access;
        access.is_open(now).then(|| access.closes_at())
    }

    /// Until when `device` is open at `now`, by itself or through one of its
    /// groups: the latest end among them, `Some(None)` without end, none
    /// while closed.
    pub fn device_until(
        &self,
        device: &Device,
        now: OffsetDateTime,
    ) -> Option<Option<OffsetDateTime>> {
        let scopes = std::iter::once(Scope::Device {
            name: device.name.clone(),
        })
        .chain(device.groups.iter().map(|group| Scope::Group {
            name: group.clone(),
        }));
        scopes
            .map(|scope| self.access.get(&scope).access)
            .filter(|access| access.is_open(now))
            .map(|access| Some(access.closes_at()))
            .fold(None, later)
    }

    /// The devices open at `now`, themselves or through one of their groups.
    pub fn open_devices(&self, now: OffsetDateTime) -> Vec<&Device> {
        self.inventory
            .devices
            .iter()
            .filter(|device| self.device_until(device, now).is_some())
            .collect()
    }

    /// The groups of the list open at `now`, with their end.
    pub fn open_groups(&self, now: OffsetDateTime) -> Vec<(&str, Option<OffsetDateTime>)> {
        self.inventory
            .groups
            .iter()
            .filter_map(|name| {
                let access = self.access.groups.get(name)?.access;
                access
                    .is_open(now)
                    .then(|| (name.as_str(), access.closes_at()))
            })
            .collect()
    }

    /// Whether anything is open: the network, or a device.
    pub fn any_open(&self, now: OffsetDateTime) -> bool {
        self.network_open(now) || !self.open_devices(now).is_empty()
    }

    /// The next point in time something open closes by itself.
    pub fn next_end(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        self.access
            .all()
            .filter(|(_, stored)| stored.access.is_open(now))
            .filter_map(|(_, stored)| stored.access.closes_at())
            .min()
    }

    /// The scopes whose time ran out and that are not closed yet.
    fn ran_out(&self, now: OffsetDateTime) -> Vec<Scope> {
        self.access
            .all()
            .filter(|(_, stored)| {
                stored.access.closes_at().is_some() && !stored.access.is_open(now)
            })
            .map(|(scope, _)| scope)
            .collect()
    }
}

/// The later of two ends of access, as [`Snapshot::device_until`] gives
/// them: none is closed and loses, `Some(None)` is without end and wins.
pub fn later(
    a: Option<Option<OffsetDateTime>>,
    b: Option<Option<OffsetDateTime>>,
) -> Option<Option<OffsetDateTime>> {
    match (a, b) {
        (None, other) | (other, None) => other,
        (Some(None), _) | (_, Some(None)) => Some(None),
        (Some(Some(a)), Some(Some(b))) => Some(Some(a.max(b))),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChangeError {
    #[error(transparent)]
    Inventory(#[from] InventoryError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn path(dir: &Path) -> PathBuf {
    dir.join("access.json")
}

fn load_access(dir: &Path) -> io::Result<AccessFile> {
    match std::fs::read(path(dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AccessFile::default()),
        Err(error) => Err(error),
    }
}

fn save_access(dir: &Path, access: &AccessFile) -> io::Result<()> {
    // Written beside it and renamed, so a reader never sees half a file.
    let temporary = dir.join("access.json.new");
    std::fs::write(&temporary, serde_json::to_vec_pretty(access)?)?;
    std::fs::rename(&temporary, path(dir))
}

/// The stored state and list; closed and empty if nothing was stored yet.
pub fn load(dir: &Path) -> io::Result<Snapshot> {
    Ok(Snapshot {
        access: load_access(dir)?,
        inventory: inventory::load(dir)?,
    })
}

/// Whole seconds: nobody opens access to the nanosecond, and the log reads
/// better without.
fn seconds(at: OffsetDateTime) -> OffsetDateTime {
    at.replace_nanosecond(0).unwrap_or(at)
}

/// Stores `access` for `scope` as changed by `by` now, and writes the change
/// to the journal. A device or group must be on the list.
pub fn change(
    dir: &Path,
    journal: &Journal,
    scope: Scope,
    access: Access,
    by: Changer,
) -> Result<(), ChangeError> {
    let list = inventory::load(dir)?;
    let known = match &scope {
        Scope::Network => true,
        Scope::Device { name } => list.device(name).is_some(),
        Scope::Group { name } => list.has_group(name),
    };
    if !known {
        let name = match &scope {
            Scope::Device { name } | Scope::Group { name } => name.clone(),
            Scope::Network => String::new(),
        };
        return Err(InventoryError::Unknown(name).into());
    }
    let access = match access {
        Access::Open { until } => Access::Open {
            until: until.map(seconds),
        },
        Access::Closed => Access::Closed,
    };
    let mut file = load_access(dir)?;
    file.set(
        &scope,
        Stored {
            access,
            changed_by: Some(by.clone()),
            changed_at: Some(seconds(OffsetDateTime::now_utc())),
        },
    );
    save_access(dir, &file)?;
    journal.append(match access {
        Access::Closed if by == Changer::Expiry => Event::Expired { scope },
        Access::Closed => Event::Closed { by, scope },
        Access::Open { until } => Event::Opened { by, until, scope },
    });
    Ok(())
}

/// Adds a device to the list, closed.
pub fn add_device(
    dir: &Path,
    journal: &Journal,
    device: Device,
    by: Changer,
) -> Result<(), ChangeError> {
    let mut list = inventory::load(dir)?;
    let event = Event::DeviceAdded {
        by,
        name: device.name.clone(),
        address: device.address.to_string(),
        ports: inventory::ports_text(&device.ports),
        groups: device.groups.clone(),
    };
    list.add_device(device)?;
    inventory::save(dir, &list)?;
    journal.append(event);
    Ok(())
}

/// Removes a device from the list; its connections end, as if it closed.
pub fn remove_device(
    dir: &Path,
    journal: &Journal,
    name: &str,
    by: Changer,
) -> Result<(), ChangeError> {
    let mut list = inventory::load(dir)?;
    list.remove_device(name)?;
    let mut file = load_access(dir)?;
    file.devices.remove(name);
    save_access(dir, &file)?;
    inventory::save(dir, &list)?;
    journal.append(Event::DeviceRemoved {
        by,
        name: name.to_owned(),
    });
    Ok(())
}

pub fn add_group(
    dir: &Path,
    journal: &Journal,
    name: &str,
    by: Changer,
) -> Result<(), ChangeError> {
    let name = inventory::valid_name(name)?;
    let mut list = inventory::load(dir)?;
    list.add_group(name.clone())?;
    inventory::save(dir, &list)?;
    journal.append(Event::GroupAdded { by, name });
    Ok(())
}

/// Removes a group, from its devices too; what it opened closes.
pub fn remove_group(
    dir: &Path,
    journal: &Journal,
    name: &str,
    by: Changer,
) -> Result<(), ChangeError> {
    let mut list = inventory::load(dir)?;
    list.remove_group(name)?;
    let mut file = load_access(dir)?;
    file.groups.remove(name);
    save_access(dir, &file)?;
    inventory::save(dir, &list)?;
    journal.append(Event::GroupRemoved {
        by,
        name: name.to_owned(),
    });
    Ok(())
}

/// The access as the running connector sees it.
pub struct Gate {
    dir: PathBuf,
    journal: Arc<Journal>,
    state: watch::Sender<Snapshot>,
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

    pub fn current(&self) -> Snapshot {
        self.state.borrow().clone()
    }

    /// Sees every change from now on.
    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.state.subscribe()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Changes the access of a scope, as the web interface does.
    pub fn set(&self, scope: Scope, access: Access, by: Changer) -> Result<(), ChangeError> {
        change(&self.dir, &self.journal, scope, access, by)?;
        self.reload();
        Ok(())
    }

    /// Reads the files again after a change made here.
    pub fn reload(&self) {
        match load(&self.dir) {
            Ok(snapshot) => {
                self.state.send_replace(snapshot);
            }
            Err(error) => tracing::warn!(%error, "cannot read the access state"),
        }
    }

    /// Runs until the process ends: takes over changes from the command line,
    /// and closes what is open when its time runs out.
    pub async fn follow(self: Arc<Self>) {
        let mut tick = tokio::time::interval(FOLLOW);
        loop {
            tick.tick().await;
            match load(&self.dir) {
                Ok(snapshot) => {
                    let taken = self.state.send_if_modified(|current| {
                        let changed = *current != snapshot;
                        *current = snapshot.clone();
                        changed
                    });
                    // The command line's own process wrote the journal; the
                    // log output of the running connector says it, too.
                    if taken {
                        tracing::info!(network = ?snapshot.access.network.access, "access changed");
                    }
                }
                Err(error) => tracing::warn!(%error, "cannot read the access state"),
            }
            for scope in self.current().ran_out(OffsetDateTime::now_utc()) {
                if let Err(error) = self.set(scope, Access::Closed, Changer::Expiry) {
                    tracing::warn!(%error, "cannot close what ran out");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
