//! Configuration from the environment.
//!
//! Every setting `REMOTEHUB_X` can also be given as `REMOTEHUB_X_FILE`, the
//! path of a file holding the value (Docker secrets). Secrets such as the
//! database URL belong in files in production; the file wins if both are
//! set. The directory is not configured here but on the settings page (#144).

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use remotehub_gateway::guacamole::KEYBOARD_LAYOUTS;
use secrecy::SecretString;
use thiserror::Error;

use crate::proxy::Network;

const DEFAULT_LISTEN: &str = "0.0.0.0:8080";
/// The guacd service of the ops package's compose file.
const DEFAULT_GUACD: &str = "guacd:4822";
/// The browser service of the ops package's compose file.
const DEFAULT_BROWSER: &str = "browser:4823";

pub struct Config {
    /// Address the HTTP server binds to.
    pub listen: SocketAddr,
    /// PostgreSQL connection URL; may contain the password, never logged.
    pub database_url: String,
    /// The database password, if not in the URL. As a file, it can be the
    /// same Docker secret that PostgreSQL reads (`POSTGRES_PASSWORD_FILE`).
    pub database_password: Option<SecretString>,
    /// Built SPA (web/build) to serve; in development Vite serves the UI.
    pub web_dir: Option<PathBuf>,
    /// Log as JSON lines instead of human-readable text.
    pub log_json: bool,
    /// Origin under which people open remotehub (`https://remotehub.example.com`).
    /// Requests that change state must come from it.
    pub public_origin: String,
    pub session: SessionConfig,
    /// File with the vault's master keys (a Docker secret), never an
    /// environment variable.
    pub master_key_file: PathBuf,
    /// File with the SSH CA's private key (a Docker secret); without it no
    /// device signs in with a certificate.
    pub ssh_ca_key_file: Option<PathBuf>,
    /// Keep the sign-in password, encrypted with a key only in the user's
    /// cookie, so devices can be opened with the own directory account.
    pub own_account_connections: bool,
    /// guacd for RDP and VNC (`host:port`), only reachable on an internal
    /// network.
    pub guacd: String,
    /// The browser service for HTTPS devices (`host:port`), only reachable
    /// on an internal network, like guacd.
    pub browser: String,
    /// Keyboard layout of RDP sessions for devices without one of their own.
    pub rdp_keyboard_layout: String,
    /// Reverse proxies whose `X-Forwarded-For` names the client.
    pub trusted_proxies: Vec<Network>,
    /// Ory Kratos for local accounts (#103); without it, only AD and
    /// break-glass accounts sign in.
    pub kratos: Option<KratosConfig>,
}

/// Where remotehub reaches Kratos, on an internal network like guacd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KratosConfig {
    /// The public API (`http://kratos:4433`); browsers reach it only
    /// through remotehub.
    pub public_url: String,
    /// The admin API (`http://kratos:4434`); never reachable from outside.
    pub admin_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// A session ends after this long without a request.
    pub idle: Duration,
    /// A session ends this long after sign-in, whatever happens.
    pub max: Duration,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{0} is not set (also possible as {0}_FILE)")]
    Missing(&'static str),
    #[error("{0} is not set: the path of a file, never the value itself")]
    MissingFile(&'static str),
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
        let required = |name: &'static str| setting(name)?.ok_or(ConfigError::Missing(name));

        let listen = listen_address(&lookup)?;
        let (database_url, database_password) = database(&lookup)?;
        let web_dir = setting("REMOTEHUB_WEB_DIR")?.map(PathBuf::from);

        let log_json = match setting("REMOTEHUB_LOG_FORMAT")?.as_deref() {
            None | Some("text") => false,
            Some("json") => true,
            Some(other) => return Err(invalid("REMOTEHUB_LOG_FORMAT", other)),
        };

        let public_url = required("REMOTEHUB_PUBLIC_URL")?;
        let public_origin =
            origin(&public_url).ok_or_else(|| invalid("REMOTEHUB_PUBLIC_URL", &public_url))?;

        let idle_minutes: u64 = parse_or(
            "REMOTEHUB_SESSION_IDLE_MINUTES",
            setting("REMOTEHUB_SESSION_IDLE_MINUTES")?,
            30,
        )?;
        let max_hours: u64 = parse_or(
            "REMOTEHUB_SESSION_MAX_HOURS",
            setting("REMOTEHUB_SESSION_MAX_HOURS")?,
            12,
        )?;
        if idle_minutes == 0 || max_hours == 0 || idle_minutes > max_hours * 60 {
            return Err(invalid(
                "REMOTEHUB_SESSION_IDLE_MINUTES",
                &idle_minutes.to_string(),
            ));
        }
        let session = SessionConfig {
            idle: Duration::from_secs(idle_minutes * 60),
            max: Duration::from_secs(max_hours * 3600),
        };

        // Only as a path: the key itself must never sit in the environment.
        let master_key_file = PathBuf::from(
            lookup("REMOTEHUB_MASTER_KEY_FILE")
                .filter(|p| !p.is_empty())
                .ok_or(ConfigError::MissingFile("REMOTEHUB_MASTER_KEY_FILE"))?,
        );
        // Like the master key: only as a path.
        let ssh_ca_key_file = lookup("REMOTEHUB_SSH_CA_KEY_FILE")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);

        let kratos = match setting("REMOTEHUB_KRATOS_URL")? {
            None => None,
            Some(public_url) => {
                let admin_url = required("REMOTEHUB_KRATOS_ADMIN_URL")?;
                let service = |name: &'static str, url: String| {
                    if url.starts_with("http://") || url.starts_with("https://") {
                        Ok(url.trim_end_matches('/').to_owned())
                    } else {
                        Err(invalid(name, &url))
                    }
                };
                Some(KratosConfig {
                    public_url: service("REMOTEHUB_KRATOS_URL", public_url)?,
                    admin_url: service("REMOTEHUB_KRATOS_ADMIN_URL", admin_url)?,
                })
            }
        };

        let own_account_connections = parse_or(
            "REMOTEHUB_OWN_ACCOUNT_CONNECTIONS",
            setting("REMOTEHUB_OWN_ACCOUNT_CONNECTIONS")?,
            true,
        )?;

        let guacd = setting("REMOTEHUB_GUACD")?.unwrap_or_else(|| DEFAULT_GUACD.to_owned());
        if !guacd.contains(':') || guacd.contains(['/', ' ']) {
            return Err(invalid("REMOTEHUB_GUACD", &guacd));
        }
        let browser = setting("REMOTEHUB_BROWSER")?.unwrap_or_else(|| DEFAULT_BROWSER.to_owned());
        if !browser.contains(':') || browser.contains(['/', ' ']) {
            return Err(invalid("REMOTEHUB_BROWSER", &browser));
        }

        let rdp_keyboard_layout =
            setting("REMOTEHUB_RDP_KEYBOARD_LAYOUT")?.unwrap_or_else(|| "en-us-qwerty".to_owned());
        if !KEYBOARD_LAYOUTS.contains(&rdp_keyboard_layout.as_str()) {
            return Err(invalid(
                "REMOTEHUB_RDP_KEYBOARD_LAYOUT",
                &rdp_keyboard_layout,
            ));
        }

        let trusted_proxies = setting("REMOTEHUB_TRUSTED_PROXIES")?
            .map(|list| {
                list.split([',', ' '])
                    .filter(|entry| !entry.is_empty())
                    .map(|entry| {
                        entry
                            .parse()
                            .map_err(|()| invalid("REMOTEHUB_TRUSTED_PROXIES", entry))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        Ok(Config {
            listen,
            database_url,
            database_password,
            web_dir,
            log_json,
            public_origin,
            session,
            master_key_file,
            ssh_ca_key_file,
            own_account_connections,
            guacd,
            browser,
            rdp_keyboard_layout,
            trusted_proxies,
            kratos,
        })
    }
}

/// The address the server binds to (`REMOTEHUB_LISTEN`). The health check
/// reads it alone, without the settings a running server needs.
/// The database's URL and password alone, for a command that needs nothing
/// else: the recovery of a lost master key file (#96).
pub fn database(
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<(String, Option<SecretString>), ConfigError> {
    let url = read_setting(lookup, "REMOTEHUB_DATABASE_URL")?
        .ok_or(ConfigError::Missing("REMOTEHUB_DATABASE_URL"))?;
    let password = read_setting(lookup, "REMOTEHUB_DATABASE_PASSWORD")?.map(SecretString::from);
    Ok((url, password))
}

pub fn listen_address(lookup: &impl Fn(&str) -> Option<String>) -> Result<SocketAddr, ConfigError> {
    parse_or(
        "REMOTEHUB_LISTEN",
        read_setting(lookup, "REMOTEHUB_LISTEN")?,
        DEFAULT_LISTEN.parse().unwrap(),
    )
}

/// `https://host[:port]` without path; the scheme and host are lowercased.
fn origin(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let valid = matches!(scheme.as_str(), "http" | "https")
        && !rest.is_empty()
        && !rest.contains(['/', '?', '#', '@', ' ']);
    valid.then(|| format!("{scheme}://{}", rest.to_ascii_lowercase()))
}

fn invalid(name: &'static str, value: &str) -> ConfigError {
    ConfigError::Invalid {
        name,
        value: value.to_owned(),
    }
}

fn parse_or<T: FromStr>(
    name: &'static str,
    value: Option<String>,
    default: T,
) -> Result<T, ConfigError> {
    match value {
        None => Ok(default),
        Some(value) => value.parse().map_err(|_| invalid(name, &value)),
    }
}

/// `NAME_FILE` (trimmed file content) takes precedence over `NAME`; empty
/// values count as unset.
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

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("listen", &self.listen)
            .field("database_url", &"<redacted>")
            .field("database_password", &"<redacted>")
            .field("web_dir", &self.web_dir)
            .field("log_json", &self.log_json)
            .field("public_origin", &self.public_origin)
            .field("session", &self.session)
            .field("master_key_file", &self.master_key_file)
            .field("ssh_ca_key_file", &self.ssh_ca_key_file)
            .field("guacd", &self.guacd)
            .field("browser", &self.browser)
            .field("rdp_keyboard_layout", &self.rdp_keyboard_layout)
            .field("trusted_proxies", &self.trusted_proxies)
            .field("kratos", &self.kratos)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;

    use super::*;

    const BASE: [(&str, &str); 3] = [
        ("REMOTEHUB_DATABASE_URL", "postgres://db/x"),
        ("REMOTEHUB_PUBLIC_URL", "https://remotehub.example.com"),
        (
            "REMOTEHUB_MASTER_KEY_FILE",
            "/run/secrets/remotehub-master-key",
        ),
    ];

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = BASE
            .iter()
            .chain(pairs)
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn without(name: &str) -> impl Fn(&str) -> Option<String> + use<'_> {
        let base = lookup(&[]);
        move |n| if n == name { None } else { base(n) }
    }

    #[test]
    fn uses_defaults_and_requires_database_and_public_url() {
        let config = Config::from_lookup(lookup(&[])).unwrap();
        assert_eq!(config.listen, DEFAULT_LISTEN.parse().unwrap());
        assert_eq!(config.database_url, "postgres://db/x");
        assert_eq!(config.public_origin, "https://remotehub.example.com");
        assert_eq!(config.session.idle, Duration::from_secs(30 * 60));
        assert_eq!(config.session.max, Duration::from_secs(12 * 3600));
        assert_eq!(config.guacd, "guacd:4822");
        assert_eq!(config.browser, "browser:4823");
        assert_eq!(config.rdp_keyboard_layout, "en-us-qwerty");
        assert!(!config.log_json);

        assert_eq!(
            Config::from_lookup(without("REMOTEHUB_MASTER_KEY_FILE")).unwrap_err(),
            ConfigError::MissingFile("REMOTEHUB_MASTER_KEY_FILE")
        );
        for name in ["REMOTEHUB_DATABASE_URL", "REMOTEHUB_PUBLIC_URL"] {
            assert_eq!(
                Config::from_lookup(without(name)).unwrap_err(),
                ConfigError::Missing(name)
            );
        }
    }

    #[test]
    fn reads_kratos_with_both_of_its_apis() {
        assert!(Config::from_lookup(lookup(&[])).unwrap().kratos.is_none());
        let config = Config::from_lookup(lookup(&[
            ("REMOTEHUB_KRATOS_URL", "http://kratos:4433/"),
            ("REMOTEHUB_KRATOS_ADMIN_URL", "http://kratos:4434"),
        ]))
        .unwrap();
        assert_eq!(
            config.kratos,
            Some(KratosConfig {
                public_url: "http://kratos:4433".to_owned(),
                admin_url: "http://kratos:4434".to_owned(),
            })
        );
        assert_eq!(
            Config::from_lookup(lookup(&[("REMOTEHUB_KRATOS_URL", "http://kratos:4433")]))
                .unwrap_err(),
            ConfigError::Missing("REMOTEHUB_KRATOS_ADMIN_URL")
        );
        assert!(matches!(
            Config::from_lookup(lookup(&[
                ("REMOTEHUB_KRATOS_URL", "kratos:4433"),
                ("REMOTEHUB_KRATOS_ADMIN_URL", "http://kratos:4434"),
            ])),
            Err(ConfigError::Invalid {
                name: "REMOTEHUB_KRATOS_URL",
                ..
            })
        ));
    }

    #[test]
    fn lists_trusted_proxies() {
        let config = Config::from_lookup(lookup(&[(
            "REMOTEHUB_TRUSTED_PROXIES",
            "10.213.213.1, fd00::/8,,",
        )]))
        .unwrap();
        let listed: Vec<String> = config
            .trusted_proxies
            .iter()
            .map(|n| n.to_string())
            .collect();
        assert_eq!(listed, ["10.213.213.1/32", "fd00::/8"]);
        assert!(
            Config::from_lookup(lookup(&[]))
                .unwrap()
                .trusted_proxies
                .is_empty()
        );
        assert_eq!(
            Config::from_lookup(lookup(&[("REMOTEHUB_TRUSTED_PROXIES", "10.0.0.1,proxy")]))
                .unwrap_err(),
            invalid("REMOTEHUB_TRUSTED_PROXIES", "proxy")
        );
    }

    #[test]
    fn normalises_the_public_origin() {
        assert_eq!(
            origin("HTTPS://RemoteHub.Example.com:8443/"),
            Some("https://remotehub.example.com:8443".into())
        );
        assert_eq!(
            origin("http://localhost:5180"),
            Some("http://localhost:5180".into())
        );
        for bad in [
            "remotehub.example.com",
            "ftp://x",
            "https://",
            "https://x/path",
            "https://u@x",
        ] {
            assert_eq!(origin(bad), None, "{bad}");
        }
    }

    #[test]
    fn reads_secrets_from_files_first() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "postgres://from-file/x").unwrap();
        let path = file.path().to_str().unwrap();

        let config = Config::from_lookup(lookup(&[("REMOTEHUB_DATABASE_URL_FILE", path)])).unwrap();
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

        for (name, value) in [
            ("REMOTEHUB_LISTEN", "not an address"),
            ("REMOTEHUB_SESSION_IDLE_MINUTES", "0"),
            ("REMOTEHUB_SESSION_IDLE_MINUTES", "soon"),
            ("REMOTEHUB_PUBLIC_URL", "remotehub.example.com"),
            ("REMOTEHUB_GUACD", "guacd"),
            ("REMOTEHUB_GUACD", "tcp://guacd:4822"),
            ("REMOTEHUB_BROWSER", "browser"),
            ("REMOTEHUB_RDP_KEYBOARD_LAYOUT", "de"),
        ] {
            assert!(
                matches!(
                    Config::from_lookup(lookup(&[(name, value)])),
                    Err(ConfigError::Invalid { .. })
                ),
                "{name}={value}"
            );
        }
    }

    #[test]
    fn never_prints_the_database_url() {
        let config = Config::from_lookup(lookup(&[(
            "REMOTEHUB_DATABASE_URL",
            "postgres://u:hunter2@db/x",
        )]))
        .unwrap();
        assert!(!format!("{config:?}").contains("hunter2"));
    }
}
