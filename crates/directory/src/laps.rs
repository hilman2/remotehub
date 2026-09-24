//! Local administrator passwords that LAPS keeps in Active Directory (#18),
//! read when a connection needs them and never stored by remotehub.
//!
//! Windows LAPS in plain-text mode keeps `msLAPS-Password` on the computer
//! account: JSON with the account (`n`), the time it was set (`t`) and the
//! password (`p`). Legacy Microsoft LAPS keeps `ms-Mcs-AdmPwd`, the bare
//! password of the built-in administrator. Encrypted Windows LAPS
//! (`msLAPS-EncryptedPassword`) and Entra LAPS are not read yet.

use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

use crate::AuthError;

/// The attributes to ask for; `sAMAccountName` names the computer as the
/// domain of its local accounts.
pub const ATTRIBUTES: [&str; 4] = [
    "sAMAccountName",
    "dNSHostName",
    "msLAPS-Password",
    "ms-Mcs-AdmPwd",
];

/// The account legacy LAPS manages unless a policy names another.
const LEGACY_ACCOUNT: &str = "Administrator";

/// A local account and its current LAPS password.
pub struct LapsPassword {
    /// e.g. `Administrator`
    pub account: String,
    pub password: SecretString,
    /// The computer's NetBIOS name, which is the domain of its local accounts.
    pub computer: String,
}

#[derive(Debug, Error)]
pub enum LapsError {
    #[error("no computer account for {0}")]
    ComputerNotFound(String),
    #[error("more than one computer account for {0}")]
    Ambiguous(String),
    /// Either LAPS does not manage the computer, or the service account may
    /// not read the attribute (AD hides it without an error).
    #[error("the computer account of {0} holds no LAPS password that remotehub may read")]
    NoPassword(String),
    #[error(transparent)]
    Directory(#[from] AuthError),
}

#[derive(Deserialize)]
struct WindowsLaps {
    #[serde(rename = "n")]
    account: String,
    #[serde(rename = "p")]
    password: String,
}

/// The password from Windows LAPS's `msLAPS-Password` or, failing that,
/// legacy LAPS's `ms-Mcs-AdmPwd`; `None` if neither holds one.
pub fn password(
    windows: Option<&str>,
    legacy: Option<&str>,
    computer: &str,
) -> Option<LapsPassword> {
    let from_windows = windows
        .and_then(|json| serde_json::from_str::<WindowsLaps>(json).ok())
        .filter(|laps| !laps.account.is_empty() && !laps.password.is_empty())
        .map(|laps| (laps.account, laps.password));
    let from_legacy = legacy
        .filter(|password| !password.is_empty())
        .map(|password| (LEGACY_ACCOUNT.to_owned(), password.to_owned()));
    let (account, password) = from_windows.or(from_legacy)?;
    Some(LapsPassword {
        account,
        password: SecretString::from(password),
        computer: computer.trim_end_matches('$').to_owned(),
    })
}

/// The LDAP filter for the computer accounts `host` may name: its full DNS
/// name, or every DNS name whose first label it is. `None` for an IP
/// address, which names no computer account.
pub fn filter(host: &str) -> Option<String> {
    let host = normalize(host);
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let name = ldap3::ldap_escape(host);
    let label = ldap3::ldap_escape(host.split('.').next().unwrap_or(host));
    Some(format!(
        "(&(objectCategory=computer)(|(dNSHostName={name})(dNSHostName={label}.*)))"
    ))
}

/// Which of the computers found for `host` (by their `dNSHostName`) it
/// means: the only one, or the one whose full name it is.
pub fn choose(dns_names: &[Option<String>], host: &str) -> Result<usize, LapsError> {
    let host = normalize(host);
    match dns_names.len() {
        0 => Err(LapsError::ComputerNotFound(host.into())),
        1 => Ok(0),
        _ => {
            let mut exact = dns_names.iter().enumerate().filter(|(_, name)| {
                name.as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(host))
            });
            match (exact.next(), exact.next()) {
                (Some((index, _)), None) => Ok(index),
                _ => Err(LapsError::Ambiguous(host.into())),
            }
        }
    }
}

fn normalize(host: &str) -> &str {
    host.trim().trim_end_matches('.')
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn windows_laps_names_its_account() {
        let json = r#"{"n":"LocalAdmin","t":"1d8b2c3d4e5f6a7","p":"S3cret!{}\"x"}"#;
        let laps = password(Some(json), Some("legacy"), "WEB01$").unwrap();
        assert_eq!(laps.account, "LocalAdmin");
        assert_eq!(laps.password.expose_secret(), "S3cret!{}\"x");
        assert_eq!(laps.computer, "WEB01");
    }

    #[test]
    fn legacy_laps_is_for_the_administrator() {
        let laps = password(None, Some("Legacy-Pw"), "WEB01$").unwrap();
        assert_eq!(laps.account, "Administrator");
        assert_eq!(laps.password.expose_secret(), "Legacy-Pw");
        // Unreadable Windows LAPS data falls back to legacy LAPS.
        let laps = password(Some("not json"), Some("Legacy-Pw"), "WEB01$").unwrap();
        assert_eq!(laps.account, "Administrator");
    }

    #[test]
    fn without_a_password_there_is_none() {
        assert!(password(None, None, "WEB01$").is_none());
        assert!(password(Some(r#"{"n":"a","p":""}"#), Some(""), "WEB01$").is_none());
        assert!(password(Some(r#"{"n":"a"}"#), None, "WEB01$").is_none());
    }

    #[test]
    fn hosts_find_their_computer_account() {
        assert_eq!(
            filter("web01.example.com").as_deref(),
            Some(
                "(&(objectCategory=computer)(|(dNSHostName=web01.example.com)(dNSHostName=web01.*)))"
            )
        );
        assert_eq!(filter("web01.").as_deref(), filter("web01").as_deref());
        assert!(filter("10.0.0.5").is_none());
        assert!(filter("fe80::1").is_none());
        assert!(filter(" ").is_none());
        // A host that is an LDAP filter stays a value.
        let hostile = filter("x*)(objectClass=*").unwrap();
        assert!(hostile.contains(r"x\2a\29\28objectClass=\2a"), "{hostile}");
    }

    #[test]
    fn a_short_name_in_two_domains_needs_the_full_one() {
        let names = |list: &[&str]| -> Vec<Option<String>> {
            list.iter().map(|n| Some((*n).to_owned())).collect()
        };
        let two = names(&["web01.a.example", "web01.b.example"]);
        assert!(matches!(
            choose(&two, "web01"),
            Err(LapsError::Ambiguous(_))
        ));
        assert_eq!(choose(&two, "WEB01.B.example.").unwrap(), 1);
        assert_eq!(choose(&names(&["web01.a.example"]), "web01").unwrap(), 0);
        assert!(matches!(
            choose(&[], "web01"),
            Err(LapsError::ComputerNotFound(_))
        ));
        assert!(matches!(
            choose(&[None, None], "web01"),
            Err(LapsError::Ambiguous(_))
        ));
    }
}
