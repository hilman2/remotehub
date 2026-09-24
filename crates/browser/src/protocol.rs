//! The agent's wire protocol: JSON lines over one TCP connection per session.
//!
//! remotehub sends one [`Open`] line. The agent answers [`Reply::Ready`] (or
//! [`Reply::Failed`] and closes), later [`Reply::Filled`] or
//! [`Reply::NotFilled`] if a login was given. The session lasts as long as
//! the connection: closing it ends the browser, and the agent closes it when
//! the browser ends.

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// Longest line either side accepts.
pub const MAX_LINE: usize = 64 * 1024;

/// Opens the web interface at `https://host:port/`.
#[derive(Deserialize)]
pub struct Open {
    pub host: String,
    pub port: u16,
    /// Where the proxy connects instead of `host:port` (`address:port`): a
    /// forward of remotehub to a device behind a site connector (ADR 0008).
    /// Chromium still sees the device's name.
    #[serde(default)]
    pub via: Option<String>,
    /// Base64 SHA-256 of the public key of the certificate remotehub pinned:
    /// Chromium accepts that certificate even when self-signed, and no other
    /// one that does not validate.
    pub spki: String,
    pub width: u32,
    pub height: u32,
    /// IANA name for the browser's clock, e.g. `Europe/Berlin`.
    pub timezone: Option<String>,
    /// Filled into the page's sign-in form; none opens the page as it is.
    pub login: Option<Login>,
}

#[derive(Deserialize)]
pub struct Login {
    pub username: String,
    pub password: SecretString,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    /// The display to open through guacd, on the agent's host.
    Ready { vnc_port: u16, vnc_password: String },
    /// Nothing was started; the connection closes.
    Failed { reason: String },
    /// The sign-in form was filled in and submitted.
    Filled,
    /// No sign-in form showed up, or typing failed; the page stays open.
    NotFilled { reason: String },
}

impl Open {
    /// The authority Chromium asks the proxy for and the page's URL are
    /// built from, lower case, IPv6 addresses in brackets.
    pub fn authority(&self) -> String {
        let host = self.host.to_ascii_lowercase();
        if host.contains(':') {
            format!("[{host}]:{}", self.port)
        } else {
            format!("{host}:{}", self.port)
        }
    }

    pub fn url(&self) -> String {
        format!("https://{}/", self.authority())
    }

    /// What the page reports as `location.origin`: without the default port.
    pub fn origin(&self) -> String {
        match self.port {
            443 => format!("https://{}", self.authority().trim_end_matches(":443")),
            _ => format!("https://{}", self.authority()),
        }
    }

    /// A host name or address: it becomes part of a URL and of Chromium's
    /// command line.
    pub fn valid_host(&self) -> bool {
        let host = self.host.as_str();
        !host.is_empty()
            && host.len() <= 253
            && !host.starts_with(['-', '.'])
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
    }
}

/// An IANA time zone name as the browser reported it; anything else is
/// dropped rather than handed to Chromium's environment.
pub fn timezone(value: Option<&str>) -> Option<&str> {
    value.filter(|zone| {
        !zone.is_empty()
            && zone.len() <= 64
            && zone
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(host: &str, port: u16) -> Open {
        Open {
            host: host.into(),
            port,
            via: None,
            spki: String::new(),
            width: 1280,
            height: 800,
            timezone: None,
            login: None,
        }
    }

    #[test]
    fn the_url_and_origin_name_the_device() {
        assert_eq!(
            open("iDRAC.example", 443).url(),
            "https://idrac.example:443/"
        );
        assert_eq!(open("iDRAC.example", 443).origin(), "https://idrac.example");
        assert_eq!(open("10.0.0.5", 8443).origin(), "https://10.0.0.5:8443");
        assert_eq!(open("fd00::5", 443).authority(), "[fd00::5]:443");
        assert_eq!(open("fd00::5", 443).origin(), "https://[fd00::5]");
    }

    #[test]
    fn hosts_are_names_or_addresses() {
        for good in ["web-target", "10.0.0.5", "fd00::5", "switch_01.lan"] {
            assert!(open(good, 443).valid_host(), "{good}");
        }
        for bad in ["", "a b", "a/b", "a@b", "--flag", "x?y", &"a".repeat(254)] {
            assert!(!open(bad, 443).valid_host(), "{bad}");
        }
    }
}
