//! Configuration from the environment.
//!
//! Every setting `REMOTEHUB_X` can also be given as `REMOTEHUB_X_FILE`, the
//! path of a file holding the value (Docker secrets). Secrets such as the
//! database URL and the LDAP password belong in files in production; the
//! file wins if both are set.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use remotehub_directory::ldap::LdapConfig;
use remotehub_gateway::guacamole::KEYBOARD_LAYOUTS;
use secrecy::SecretString;
use thiserror::Error;

const DEFAULT_LISTEN: &str = "0.0.0.0:8080";
/// The guacd service of the ops package's compose file.
const DEFAULT_GUACD: &str = "guacd:4822";

pub struct Config {
    /// Address the HTTP server binds to.
    pub listen: SocketAddr,
    /// PostgreSQL connection URL; contains the password, never logged.
    pub database_url: String,
    /// Built SPA (web/build) to serve; in development Vite serves the UI.
    pub web_dir: Option<PathBuf>,
    /// Log as JSON lines instead of human-readable text.
    pub log_json: bool,
    /// Origin under which people open remotehub (`https://remotehub.example.com`).
    /// Requests that change state must come from it.
    pub public_origin: String,
    pub session: SessionConfig,
    /// Active Directory; without it only break-glass accounts can sign in.
    pub ldap: Option<LdapConfig>,
    /// File with the vault's master keys (a Docker secret), never an
    /// environment variable.
    pub master_key_file: PathBuf,
    /// Keep the sign-in password, encrypted with a key only in the user's
    /// cookie, so devices can be opened with the own directory account.
    pub own_account_connections: bool,
    /// Groups whose members administer remotehub: SIDs or group names
    /// (names are looked up in the directory at startup).
    pub admin_groups: Vec<String>,
    /// guacd for RDP and VNC (`host:port`), only reachable on an internal
    /// network.
    pub guacd: String,
    /// Keyboard layout of RDP sessions for devices without one of their own.
    pub rdp_keyboard_layout: String,
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

        let listen = parse_or(
            "REMOTEHUB_LISTEN",
            setting("REMOTEHUB_LISTEN")?,
            DEFAULT_LISTEN.parse().unwrap(),
        )?;
        let database_url = required("REMOTEHUB_DATABASE_URL")?;
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

        let ldap = match setting("REMOTEHUB_LDAP_URL")? {
            None => None,
            Some(url) => {
                if !(url.starts_with("ldaps://") || url.starts_with("ldap://")) {
                    return Err(invalid("REMOTEHUB_LDAP_URL", &url));
                }
                let timeout: u64 = parse_or(
                    "REMOTEHUB_LDAP_TIMEOUT_SECONDS",
                    setting("REMOTEHUB_LDAP_TIMEOUT_SECONDS")?,
                    10,
                )?;
                Some(LdapConfig {
                    starttls: parse_or(
                        "REMOTEHUB_LDAP_STARTTLS",
                        setting("REMOTEHUB_LDAP_STARTTLS")?,
                        false,
                    )?,
                    url,
                    ca_file: setting("REMOTEHUB_LDAP_CA_FILE")?.map(PathBuf::from),
                    bind_dn: required("REMOTEHUB_LDAP_BIND_DN")?,
                    bind_password: SecretString::from(required("REMOTEHUB_LDAP_BIND_PASSWORD")?),
                    base_dn: required("REMOTEHUB_LDAP_BASE_DN")?,
                    user_filter: setting("REMOTEHUB_LDAP_USER_FILTER")?,
                    timeout: Duration::from_secs(timeout.max(1)),
                })
            }
        };
        if let Some(ldap) = &ldap
            && ldap.url.starts_with("ldap://")
            && !ldap.starttls
        {
            // Passwords never travel unencrypted.
            return Err(invalid("REMOTEHUB_LDAP_STARTTLS", "false with ldap://"));
        }

        // Only as a path: the key itself must never sit in the environment.
        let master_key_file = PathBuf::from(
            lookup("REMOTEHUB_MASTER_KEY_FILE")
                .filter(|p| !p.is_empty())
                .ok_or(ConfigError::MissingFile("REMOTEHUB_MASTER_KEY_FILE"))?,
        );

        let admin_groups = setting("REMOTEHUB_ADMIN_GROUPS")?
            .map(|list| {
                list.split([',', ';'])
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let own_account_connections = parse_or(
            "REMOTEHUB_OWN_ACCOUNT_CONNECTIONS",
            setting("REMOTEHUB_OWN_ACCOUNT_CONNECTIONS")?,
            true,
        )?;

        let guacd = setting("REMOTEHUB_GUACD")?.unwrap_or_else(|| DEFAULT_GUACD.to_owned());
        if !guacd.contains(':') || guacd.contains(['/', ' ']) {
            return Err(invalid("REMOTEHUB_GUACD", &guacd));
        }

        let rdp_keyboard_layout =
            setting("REMOTEHUB_RDP_KEYBOARD_LAYOUT")?.unwrap_or_else(|| "en-us-qwerty".to_owned());
        if !KEYBOARD_LAYOUTS.contains(&rdp_keyboard_layout.as_str()) {
            return Err(invalid(
                "REMOTEHUB_RDP_KEYBOARD_LAYOUT",
                &rdp_keyboard_layout,
            ));
        }

        Ok(Config {
            listen,
            database_url,
            web_dir,
            log_json,
            public_origin,
            session,
            ldap,
            master_key_file,
            own_account_connections,
            admin_groups,
            guacd,
            rdp_keyboard_layout,
        })
    }
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
            .field("public_origin", &self.public_origin)
            .field("session", &self.session)
            .field("ldap", &self.ldap.as_ref().map(|l| &l.url))
            .field("master_key_file", &self.master_key_file)
            .field("admin_groups", &self.admin_groups)
            .field("guacd", &self.guacd)
            .field("rdp_keyboard_layout", &self.rdp_keyboard_layout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;

    use secrecy::ExposeSecret;

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
        assert!(config.ldap.is_none());
        assert!(config.admin_groups.is_empty());
        assert_eq!(config.guacd, "guacd:4822");
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
    fn lists_admin_groups() {
        let config = Config::from_lookup(lookup(&[(
            "REMOTEHUB_ADMIN_GROUPS",
            " RH Admins , S-1-5-21-1-2-3-512;;",
        )]))
        .unwrap();
        assert_eq!(config.admin_groups, ["RH Admins", "S-1-5-21-1-2-3-512"]);
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
    fn reads_the_ldap_settings_and_insists_on_encryption() {
        let ldap = [
            ("REMOTEHUB_LDAP_URL", "ldaps://dc.example.com"),
            ("REMOTEHUB_LDAP_BIND_DN", "svc@example.com"),
            ("REMOTEHUB_LDAP_BIND_PASSWORD", "s3cret"),
            ("REMOTEHUB_LDAP_BASE_DN", "DC=example,DC=com"),
        ];
        let config = Config::from_lookup(lookup(&ldap)).unwrap();
        let settings = config.ldap.as_ref().unwrap();
        assert_eq!(settings.url, "ldaps://dc.example.com");
        assert_eq!(settings.bind_password.expose_secret(), "s3cret");
        assert!(!settings.starttls);
        assert_eq!(settings.timeout, Duration::from_secs(10));
        assert!(!format!("{config:?}").contains("s3cret"));

        let mut plain = ldap.to_vec();
        plain[0] = ("REMOTEHUB_LDAP_URL", "ldap://dc.example.com");
        assert!(Config::from_lookup(lookup(&plain)).is_err());
        plain.push(("REMOTEHUB_LDAP_STARTTLS", "true"));
        assert!(
            Config::from_lookup(lookup(&plain))
                .unwrap()
                .ldap
                .unwrap()
                .starttls
        );

        assert_eq!(
            Config::from_lookup(lookup(&ldap[..3])).unwrap_err(),
            ConfigError::Missing("REMOTEHUB_LDAP_BASE_DN")
        );
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
