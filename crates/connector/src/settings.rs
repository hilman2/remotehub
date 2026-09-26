//! Settings from the environment: `REMOTEHUB_*`, each also as `NAME_FILE`,
//! and from [`CONF`] in the data directory. The way to remotehub is in
//! [`crate::agent::AgentSettings`].

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is not set (also possible as {0}_FILE)")]
    Missing(&'static str),
    #[error("{name}_FILE: cannot read {path}: {reason}")]
    File {
        name: &'static str,
        path: String,
        reason: String,
    },
    #[error("{name}: invalid value {value:?}")]
    Invalid { name: &'static str, value: String },
    #[error("{0} is set, so {1} must be too")]
    Pair(&'static str, &'static str),
}

/// `NAME_FILE` (trimmed file content) takes precedence over `NAME`; empty
/// values count as unset. The same rule as the server's settings.
pub(crate) fn read_setting(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<String>, ConfigError> {
    if let Some(path) = lookup(&format!("{name}_FILE")).filter(|p| !p.is_empty()) {
        let content = std::fs::read_to_string(&path).map_err(|e| ConfigError::File {
            name,
            path: path.clone(),
            reason: e.to_string(),
        })?;
        return Ok(Some(content.trim().to_owned()).filter(|v| !v.is_empty()));
    }
    Ok(lookup(name).filter(|v| !v.is_empty()))
}

pub(crate) fn invalid(name: &'static str, value: &str) -> ConfigError {
    ConfigError::Invalid {
        name,
        value: value.to_owned(),
    }
}

/// `REMOTEHUB_CONNECTOR_DATA`: access state, journal, users and the web
/// interface's certificate.
pub fn data_dir(lookup: &impl Fn(&str) -> Option<String>) -> Result<PathBuf, ConfigError> {
    Ok(read_setting(lookup, "REMOTEHUB_CONNECTOR_DATA")?
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir))
}

#[cfg(windows)]
fn default_data_dir() -> PathBuf {
    let base = std::env::var("ProgramData").unwrap_or_else(|_| "C:/ProgramData".to_owned());
    PathBuf::from(base).join("remotehub-connector")
}

#[cfg(not(windows))]
fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/remotehub-connector")
}

/// The file in the data directory that holds settings where no environment
/// is at hand, as for the Windows service (#166): one `NAME=value` per line,
/// `#` starts a comment.
pub const CONF: &str = "connector.conf";

/// The settings from the environment, and from [`CONF`] in `data` for names
/// the environment leaves unset.
pub fn lookup(data: &Path) -> impl Fn(&str) -> Option<String> + use<> {
    let conf = std::fs::read_to_string(data.join(CONF)).unwrap_or_default();
    let values = parse_conf(&conf);
    move |name: &str| {
        std::env::var(name)
            .ok()
            .or_else(|| values.get(name).cloned())
    }
}

fn parse_conf(text: &str) -> HashMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// The web interface.
pub struct UiSettings {
    /// `REMOTEHUB_CONNECTOR_LISTEN`, default `127.0.0.1:8480`; the image
    /// listens on every address, and the published port decides who reaches
    /// it.
    pub listen: SocketAddr,
    /// `REMOTEHUB_CONNECTOR_TLS_CERT_FILE` and `…_KEY_FILE`, PEM; without
    /// them, a self-signed certificate in the data directory.
    pub certificate: Option<(PathBuf, PathBuf)>,
}

impl UiSettings {
    pub fn from_env(lookup: &impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let listen = match read_setting(lookup, "REMOTEHUB_CONNECTOR_LISTEN")? {
            Some(text) => text
                .parse()
                .map_err(|_| invalid("REMOTEHUB_CONNECTOR_LISTEN", &text))?,
            None => SocketAddr::from(([127, 0, 0, 1], 8480)),
        };
        // Only the paths: the files are read when the interface starts.
        let path = |name: &str| lookup(name).filter(|value| !value.is_empty());
        const CERT: &str = "REMOTEHUB_CONNECTOR_TLS_CERT_FILE";
        const KEY: &str = "REMOTEHUB_CONNECTOR_TLS_KEY_FILE";
        let certificate = match (path(CERT), path(KEY)) {
            (Some(cert), Some(key)) => Some((PathBuf::from(cert), PathBuf::from(key))),
            (None, None) => None,
            (Some(_), None) => return Err(ConfigError::Pair(CERT, KEY)),
            (None, Some(_)) => return Err(ConfigError::Pair(KEY, CERT)),
        };
        Ok(UiSettings {
            listen,
            certificate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conf_file_holds_names_and_values() {
        let values = parse_conf(
            "# written by install\n\
             REMOTEHUB_URL = https://remotehub.example.com\n\
             \n\
             REMOTEHUB_CONNECTOR_ALLOW=10.0.0.0/8,192.168.1.0/24\n\
             not a setting\n",
        );
        assert_eq!(values.len(), 2);
        assert_eq!(values["REMOTEHUB_URL"], "https://remotehub.example.com");
        assert_eq!(
            values["REMOTEHUB_CONNECTOR_ALLOW"],
            "10.0.0.0/8,192.168.1.0/24"
        );
    }
}
