//! The customer's own list of devices and groups (#180): what can be opened
//! one by one instead of the whole network. It lives in `inventory.json` in
//! the data directory and is kept in the web interface and on the command
//! line; remotehub sees none of it.

use std::fmt;
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::network::Network;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Inventory {
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub address: Address,
    pub ports: Vec<Ports>,
    #[serde(default)]
    pub groups: Vec<String>,
}

/// Where a device is: an address range (a single address included) or a
/// host name, which the connector resolves in its own network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Range(Network),
    Host(String),
}

/// A port or a range of ports, `22` or `8000-8100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ports {
    pub first: u16,
    pub last: u16,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InventoryError {
    #[error("a name has 1 to 64 characters and no control characters")]
    InvalidName,
    #[error("{0:?} is neither an address, an address range nor a host name")]
    InvalidAddress(String),
    #[error("{0:?} is not a port or a range of ports such as 22 or 8000-8100")]
    InvalidPorts(String),
    #[error("a device needs at least one port")]
    NoPorts,
    #[error("{0} exists already")]
    Exists(String),
    #[error("there is no {0}")]
    Unknown(String),
}

impl Address {
    /// Whether a connection to `host`, resolved to `ip`, goes to this
    /// device. `resolved` are the addresses the device's host name has in
    /// this network, if it has one.
    pub fn covers(&self, host: &str, ip: IpAddr, resolved: &[IpAddr]) -> bool {
        match self {
            Address::Range(network) => network.contains(ip),
            Address::Host(name) => name.eq_ignore_ascii_case(host) || resolved.contains(&ip),
        }
    }
}

impl FromStr for Address {
    type Err = InventoryError;

    fn from_str(text: &str) -> Result<Self, InventoryError> {
        let text = text.trim();
        if let Ok(network) = text.parse::<Network>() {
            return Ok(Address::Range(network));
        }
        let host = text.trim_end_matches('.');
        let valid = (1..=253).contains(&host.len())
            && host.split('.').all(|label| {
                (1..=63).contains(&label.len())
                    && label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    && !label.starts_with('-')
                    && !label.ends_with('-')
            });
        if valid {
            Ok(Address::Host(host.to_ascii_lowercase()))
        } else {
            Err(InventoryError::InvalidAddress(text.to_owned()))
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // A single address without its /32 or /128.
            Address::Range(network) => {
                let text = network.to_string();
                let single = text.ends_with("/32") && !text.contains(':') || text.ends_with("/128");
                f.write_str(if single {
                    text.rsplit_once('/').map_or(text.as_str(), |(a, _)| a)
                } else {
                    &text
                })
            }
            Address::Host(name) => f.write_str(name),
        }
    }
}

impl Serialize for Address {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl Ports {
    pub fn contains(&self, port: u16) -> bool {
        (self.first..=self.last).contains(&port)
    }

    /// A list such as `22, 3389, 8000-8100`.
    pub fn parse_list(text: &str) -> Result<Vec<Ports>, InventoryError> {
        let ports = text
            .split([',', ' '])
            .filter(|part| !part.is_empty())
            .map(str::parse)
            .collect::<Result<Vec<Ports>, _>>()?;
        if ports.is_empty() {
            return Err(InventoryError::NoPorts);
        }
        Ok(ports)
    }
}

impl FromStr for Ports {
    type Err = InventoryError;

    fn from_str(text: &str) -> Result<Self, InventoryError> {
        let invalid = || InventoryError::InvalidPorts(text.to_owned());
        let port = |part: &str| {
            part.trim()
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(invalid)
        };
        let (first, last) = match text.split_once('-') {
            Some((first, last)) => (port(first)?, port(last)?),
            None => (port(text)?, port(text)?),
        };
        if first > last {
            return Err(invalid());
        }
        Ok(Ports { first, last })
    }
}

impl fmt::Display for Ports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.first == self.last {
            write!(f, "{}", self.first)
        } else {
            write!(f, "{}-{}", self.first, self.last)
        }
    }
}

impl Serialize for Ports {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Ports {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// The ports as the customer writes them, `22, 3389`.
pub fn ports_text(ports: &[Ports]) -> String {
    ports
        .iter()
        .map(Ports::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A name of a device or group: what the customer calls it.
pub fn valid_name(name: &str) -> Result<String, InventoryError> {
    let name = name.trim();
    if (1..=64).contains(&name.chars().count()) && !name.chars().any(char::is_control) {
        Ok(name.to_owned())
    } else {
        Err(InventoryError::InvalidName)
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join("inventory.json")
}

/// The stored list; empty if nothing was stored yet.
pub fn load(dir: &Path) -> io::Result<Inventory> {
    match std::fs::read(path(dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Inventory::default()),
        Err(error) => Err(error),
    }
}

pub fn save(dir: &Path, inventory: &Inventory) -> io::Result<()> {
    // Written beside it and renamed, so a reader never sees half a file.
    let temporary = dir.join("inventory.json.new");
    std::fs::write(&temporary, serde_json::to_vec_pretty(inventory)?)?;
    std::fs::rename(&temporary, path(dir))
}

impl Inventory {
    pub fn device(&self, name: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.name == name)
    }

    pub fn has_group(&self, name: &str) -> bool {
        self.groups.iter().any(|g| g == name)
    }

    /// Adds a device; its groups must exist.
    pub fn add_device(&mut self, device: Device) -> Result<(), InventoryError> {
        if self.device(&device.name).is_some() {
            return Err(InventoryError::Exists(device.name));
        }
        if device.ports.is_empty() {
            return Err(InventoryError::NoPorts);
        }
        if let Some(group) = device.groups.iter().find(|g| !self.has_group(g)) {
            return Err(InventoryError::Unknown(group.clone()));
        }
        self.devices.push(device);
        self.devices.sort_by_key(|d| d.name.to_lowercase());
        Ok(())
    }

    pub fn remove_device(&mut self, name: &str) -> Result<Device, InventoryError> {
        let index = self
            .devices
            .iter()
            .position(|d| d.name == name)
            .ok_or_else(|| InventoryError::Unknown(name.to_owned()))?;
        Ok(self.devices.remove(index))
    }

    pub fn add_group(&mut self, name: String) -> Result<(), InventoryError> {
        if self.has_group(&name) {
            return Err(InventoryError::Exists(name));
        }
        self.groups.push(name);
        self.groups.sort_by_key(|g| g.to_lowercase());
        Ok(())
    }

    /// Removes a group, and it from every device in it.
    pub fn remove_group(&mut self, name: &str) -> Result<(), InventoryError> {
        if !self.has_group(name) {
            return Err(InventoryError::Unknown(name.to_owned()));
        }
        self.groups.retain(|g| g != name);
        for device in &mut self.devices {
            device.groups.retain(|g| g != name);
        }
        Ok(())
    }

    /// The devices in a group.
    pub fn members(&self, group: &str) -> impl Iterator<Item = &Device> {
        self.devices
            .iter()
            .filter(move |d| d.groups.iter().any(|g| g == group))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_ranges_or_host_names() {
        let single: Address = "192.168.10.20".parse().unwrap();
        assert!(single.covers("x", "192.168.10.20".parse().unwrap(), &[]));
        assert!(!single.covers("x", "192.168.10.21".parse().unwrap(), &[]));
        assert_eq!(single.to_string(), "192.168.10.20");
        let range: Address = "10.0.0.0/24".parse().unwrap();
        assert!(range.covers("x", "10.0.0.99".parse().unwrap(), &[]));
        assert_eq!(range.to_string(), "10.0.0.0/24");
        let host: Address = "SQL01.corp.local.".parse().unwrap();
        assert_eq!(host.to_string(), "sql01.corp.local");
        assert!(host.covers("sql01.CORP.local", "10.9.9.9".parse().unwrap(), &[]));
        let resolved = ["10.0.0.5".parse().unwrap()];
        assert!(host.covers("10.0.0.5", "10.0.0.5".parse().unwrap(), &resolved));
        assert!(!host.covers("10.0.0.6", "10.0.0.6".parse().unwrap(), &resolved));
        for bad in ["", "a b", "-x.local", "x..local", "10.0.0.0/33", "<b>"] {
            assert!(bad.parse::<Address>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ports_are_single_or_ranges() {
        let ports = Ports::parse_list("22, 3389 8000-8100").unwrap();
        assert_eq!(ports_text(&ports), "22, 3389, 8000-8100");
        assert!(ports[2].contains(8050) && !ports[2].contains(8101));
        for bad in ["", "0", "65536", "90-80", "x", "22,,x"] {
            assert!(Ports::parse_list(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn devices_belong_to_existing_groups() {
        let mut inventory = Inventory::default();
        let device = |name: &str, groups: &[&str]| Device {
            name: name.into(),
            address: "10.0.0.1".parse().unwrap(),
            ports: Ports::parse_list("22").unwrap(),
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
        };
        assert_eq!(
            inventory.add_device(device("sql", &["ERP"])),
            Err(InventoryError::Unknown("ERP".into()))
        );
        inventory.add_group("ERP".into()).unwrap();
        inventory.add_device(device("sql", &["ERP"])).unwrap();
        inventory.add_device(device("haproxy", &["ERP"])).unwrap();
        assert_eq!(
            inventory.add_device(device("sql", &[])),
            Err(InventoryError::Exists("sql".into()))
        );
        assert_eq!(inventory.members("ERP").count(), 2);
        inventory.remove_group("ERP").unwrap();
        assert!(inventory.devices.iter().all(|d| d.groups.is_empty()));
        assert!(inventory.remove_device("sql").is_ok());
        assert!(inventory.remove_device("sql").is_err());
    }
}
