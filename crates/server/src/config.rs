//! Configuration from the environment.
//!
//! Every setting `REMOTEHUB_X` can also be given as `REMOTEHUB_X_FILE`, the
//! path of a file holding the value (Docker secrets). Secrets such as the
//! database URL belong in files in production; the file wins if both are set.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use thiserror::Error;

const DEFAULT_LISTEN: &str = "0.0.0.0:8080";

#[derive(Clone)]
pub struct Config {
    /// Address the HTTP server binds to.
    pub listen: SocketAddr,
    /// PostgreSQL connection URL; contains the password, never logged.
    pub database_url: String,
    /// Built SPA (web/build) to serve; in development Vite serves the UI.
    pub web_dir: Option<PathBuf>,
    /// Log as JSON lines instead of human-readable text.
    pub log_json: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
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
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the configuration through `lookup`, so tests need no process
    /// environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let setting = |name: &'static str| read_setting(&lookup, name);

        let listen_raw = setting("REMOTEHUB_LISTEN")?.unwrap_or_else(|| DEFAULT_LISTEN.to_owned());
        let listen = listen_raw.parse().map_err(|_| ConfigError::Invalid {
            name: "REMOTEHUB_LISTEN",
            value: listen_raw,
        })?;

        let database_url = setting("REMOTEHUB_DATABASE_URL")?
            .ok_or(ConfigError::Missing("REMOTEHUB_DATABASE_URL"))?;

        let web_dir = setting("REMOTEHUB_WEB_DIR")?.map(PathBuf::from);

        let log_json = match setting("REMOTEHUB_LOG_FORMAT")?.as_deref() {
            None | Some("text") => false,
            Some("json") => true,
            Some(other) => {
                return Err(ConfigError::Invalid {
                    name: "REMOTEHUB_LOG_FORMAT",
                    value: other.to_owned(),
                });
            }
        };

        Ok(Config {
            listen,
            database_url,
            web_dir,
            log_json,
        })
    }
}

/// `NAME_FILE` (trimmed file content) takes precedence over `NAME`; empty
/// values count as unset.
fn read_setting(
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

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("listen", &self.listen)
            .field("database_url", &"<redacted>")
            .field("web_dir", &self.web_dir)
            .field("log_json", &self.log_json)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;

    use super::*;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn uses_defaults_and_requires_the_database_url() {
        let config =
            Config::from_lookup(lookup(&[("REMOTEHUB_DATABASE_URL", "postgres://db/x")])).unwrap();
        assert_eq!(config.listen, DEFAULT_LISTEN.parse().unwrap());
        assert_eq!(config.database_url, "postgres://db/x");
        assert_eq!(config.web_dir, None);
        assert!(!config.log_json);

        assert_eq!(
            Config::from_lookup(lookup(&[])).unwrap_err(),
            ConfigError::Missing("REMOTEHUB_DATABASE_URL")
        );
    }

    #[test]
    fn reads_secrets_from_files_first() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "postgres://from-file/x").unwrap();
        let path = file.path().to_str().unwrap();

        let config = Config::from_lookup(lookup(&[
            ("REMOTEHUB_DATABASE_URL", "postgres://from-env/x"),
            ("REMOTEHUB_DATABASE_URL_FILE", path),
        ]))
        .unwrap();
        assert_eq!(config.database_url, "postgres://from-file/x");
    }

    #[test]
    fn reports_unreadable_files_and_invalid_values() {
        let err = Config::from_lookup(lookup(&[(
            "REMOTEHUB_DATABASE_URL_FILE",
            "/does/not/exist",
        )]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::File {
                name: "REMOTEHUB_DATABASE_URL",
                ..
            }
        ));

        let err = Config::from_lookup(lookup(&[
            ("REMOTEHUB_DATABASE_URL", "postgres://db/x"),
            ("REMOTEHUB_LISTEN", "not an address"),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::Invalid {
                name: "REMOTEHUB_LISTEN",
                ..
            }
        ));
    }

    #[test]
    fn never_prints_the_database_url() {
        let config = Config::from_lookup(lookup(&[(
            "REMOTEHUB_DATABASE_URL",
            "postgres://u:secret@db/x",
        )]))
        .unwrap();
        assert!(!format!("{config:?}").contains("secret"));
    }
}
