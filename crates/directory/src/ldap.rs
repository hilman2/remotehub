//! Active Directory over LDAPS (or LDAP with StartTLS).
//!
//! A sign-in takes three steps:
//! 1. The service account finds exactly one user entry for the given name
//!    (`sAMAccountName`, `DOMAIN\name` or `userPrincipalName`) below the base
//!    DN, optionally narrowed by an extra filter.
//! 2. A bind as that entry's DN on a separate connection verifies the
//!    password. Empty passwords are rejected before, because LDAP would treat
//!    them as an anonymous bind that "succeeds".
//! 3. The service account reads `tokenGroups` of the entry: the SIDs of all
//!    groups, nested ones included, computed by the domain controller.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ldap3::{
    Ldap, LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry, SearchOptions,
    SearchResult,
};
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use crate::{AuthError, Group, Identity, IdentityProvider, Sid};

/// LDAP result codes.
const SIZE_LIMIT_EXCEEDED: u32 = 4;
const INVALID_CREDENTIALS: u32 = 49;

const USER_ATTRIBUTES: [&str; 6] = [
    "sAMAccountName",
    "userPrincipalName",
    "displayName",
    "mail",
    "objectSid",
    "objectGUID",
];

pub struct LdapConfig {
    /// `ldaps://dc.example.com` or, with `starttls`, `ldap://dc.example.com`.
    pub url: String,
    pub starttls: bool,
    /// PEM file with the CA certificates that sign the domain controllers'
    /// certificates; without it the system's roots are used.
    pub ca_file: Option<PathBuf>,
    /// Service account for lookups, as DN or UPN.
    pub bind_dn: String,
    pub bind_password: SecretString,
    /// Where users and groups are searched, e.g. `DC=example,DC=com`.
    pub base_dn: String,
    /// Extra LDAP filter a user must match to sign in, e.g. membership of a
    /// group: `(memberOf:1.2.840.113556.1.4.1941:=CN=remotehub users,…)`.
    pub user_filter: Option<String>,
    /// Limit for connecting and for each operation.
    pub timeout: Duration,
}

pub struct LdapDirectory {
    config: LdapConfig,
    tls: Option<Arc<ClientConfig>>,
}

impl LdapDirectory {
    pub fn new(config: LdapConfig) -> Result<Self, AuthError> {
        let tls = config
            .ca_file
            .as_deref()
            .map(tls_config)
            .transpose()
            .map_err(AuthError::Directory)?;
        Ok(LdapDirectory { config, tls })
    }

    async fn connect(&self) -> Result<Ldap, AuthError> {
        let mut settings = LdapConnSettings::new()
            .set_conn_timeout(self.config.timeout)
            .set_starttls(self.config.starttls);
        if let Some(tls) = &self.tls {
            settings = settings.set_config(tls.clone());
        }
        let (conn, ldap) = LdapConnAsync::with_settings(settings, &self.config.url)
            .await
            .map_err(|e| AuthError::Unavailable(e.to_string()))?;
        ldap3::drive!(conn);
        Ok(ldap)
    }

    /// A connection bound as the service account.
    async fn service(&self) -> Result<Ldap, AuthError> {
        let mut ldap = self.connect().await?;
        ldap.with_timeout(self.config.timeout)
            .simple_bind(
                &self.config.bind_dn,
                self.config.bind_password.expose_secret(),
            )
            .await
            .map_err(unavailable)?
            .success()
            .map_err(|e| AuthError::Directory(format!("service account bind failed: {e}")))?;
        Ok(ldap)
    }

    /// Security groups whose name contains `query`, for choosing grantees.
    pub async fn search_groups(&self, query: &str, limit: i32) -> Result<Vec<Group>, AuthError> {
        let query = ldap3::ldap_escape(query.trim());
        let filter =
            format!("(&(objectCategory=group)(|(cn=*{query}*)(sAMAccountName=*{query}*)))");
        let mut ldap = self.service().await?;
        let SearchResult(entries, result) = ldap
            .with_timeout(self.config.timeout)
            .with_search_options(SearchOptions::new().sizelimit(limit))
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                vec!["cn", "objectSid"],
            )
            .await
            .map_err(unavailable)?;
        let _ = ldap.unbind().await;
        // More matches than the limit is fine: the first ones are enough.
        if result.rc != 0 && result.rc != SIZE_LIMIT_EXCEEDED {
            return Err(AuthError::Directory(format!(
                "group search failed with code {}: {}",
                result.rc, result.text
            )));
        }

        let mut groups: Vec<Group> = entries
            .into_iter()
            .filter_map(|entry| {
                let entry = SearchEntry::construct(entry);
                Some(Group {
                    sid: Sid::from_bytes(first_binary(&entry, "objectSid")?.as_slice()).ok()?,
                    name: first_text(&entry, "cn")?,
                })
            })
            .collect();
        groups.sort_by_key(|g| g.name.to_lowercase());
        Ok(groups)
    }

    async fn find_user(
        &self,
        ldap: &mut Ldap,
        account: &Account,
    ) -> Result<SearchEntry, AuthError> {
        let filter = user_filter(account, self.config.user_filter.as_deref());
        let (entries, _) = ldap
            .with_timeout(self.config.timeout)
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                USER_ATTRIBUTES.to_vec(),
            )
            .await
            .map_err(unavailable)?
            .success()
            .map_err(directory)?;
        match <[_; 1]>::try_from(entries) {
            Ok([entry]) => Ok(SearchEntry::construct(entry)),
            Err(entries) if entries.is_empty() => Err(AuthError::InvalidCredentials),
            Err(_) => Err(AuthError::Directory(format!(
                "more than one entry matches {filter}"
            ))),
        }
    }

    async fn verify_password(&self, dn: &str, password: &SecretString) -> Result<(), AuthError> {
        let mut ldap = self.connect().await?;
        let result = ldap
            .with_timeout(self.config.timeout)
            .simple_bind(dn, password.expose_secret())
            .await
            .map_err(unavailable)?;
        let _ = ldap.unbind().await;
        match result.rc {
            0 => Ok(()),
            INVALID_CREDENTIALS => Err(bind_failure(&result.text)),
            rc => Err(AuthError::Directory(format!(
                "bind failed with code {rc}: {}",
                result.text
            ))),
        }
    }

    async fn token_groups(&self, ldap: &mut Ldap, dn: &str) -> Result<Vec<Sid>, AuthError> {
        let (entries, _) = ldap
            .with_timeout(self.config.timeout)
            .search(dn, Scope::Base, "(objectClass=*)", vec!["tokenGroups"])
            .await
            .map_err(unavailable)?
            .success()
            .map_err(directory)?;
        let mut groups = Vec::new();
        for entry in entries {
            for value in binary_values(&SearchEntry::construct(entry), "tokenGroups") {
                groups.push(
                    Sid::from_bytes(&value)
                        .map_err(|_| AuthError::Directory("invalid SID in tokenGroups".into()))?,
                );
            }
        }
        groups.sort();
        groups.dedup();
        Ok(groups)
    }
}

impl IdentityProvider for LdapDirectory {
    async fn authenticate(
        &self,
        username: &str,
        password: &SecretString,
    ) -> Result<Identity, AuthError> {
        let account = Account::parse(username).ok_or(AuthError::InvalidCredentials)?;
        if password.expose_secret().is_empty() {
            return Err(AuthError::InvalidCredentials);
        }

        let mut service = self.service().await?;
        let entry = self.find_user(&mut service, &account).await?;
        self.verify_password(&entry.dn, password).await?;
        let groups = self.token_groups(&mut service, &entry.dn).await?;
        let _ = service.unbind().await;

        let sid = first_binary(&entry, "objectSid")
            .and_then(|b| Sid::from_bytes(&b).ok())
            .ok_or_else(|| AuthError::Directory(format!("{} has no valid objectSid", entry.dn)))?;
        let guid = first_binary(&entry, "objectGUID")
            .and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok())
            .map(Uuid::from_bytes_le)
            .ok_or_else(|| AuthError::Directory(format!("{} has no valid objectGUID", entry.dn)))?;
        let username = first_text(&entry, "sAMAccountName")
            .ok_or_else(|| AuthError::Directory(format!("{} has no sAMAccountName", entry.dn)))?;

        Ok(Identity {
            sid,
            guid,
            display_name: first_text(&entry, "displayName").unwrap_or_else(|| username.clone()),
            username,
            upn: first_text(&entry, "userPrincipalName"),
            email: first_text(&entry, "mail"),
            groups,
        })
    }
}

/// The name a user typed, reduced to what identifies the account.
#[derive(Debug, PartialEq, Eq)]
enum Account {
    /// `alice` or `DOMAIN\alice` (single domain: the prefix is dropped).
    Sam(String),
    /// `alice@example.com`
    Upn(String),
}

impl Account {
    fn parse(input: &str) -> Option<Account> {
        let input = input.trim();
        if input.is_empty() || input.len() > 256 || input.chars().any(char::is_control) {
            return None;
        }
        let account = match input.split_once('\\') {
            Some((_, name)) => Account::Sam(name.to_owned()),
            None if input.contains('@') => Account::Upn(input.to_owned()),
            None => Account::Sam(input.to_owned()),
        };
        match &account {
            Account::Sam(name) | Account::Upn(name) if name.is_empty() => None,
            _ => Some(account),
        }
    }
}

fn user_filter(account: &Account, extra: Option<&str>) -> String {
    let name = match account {
        Account::Sam(name) => format!("(sAMAccountName={})", ldap3::ldap_escape(name)),
        Account::Upn(upn) => format!("(userPrincipalName={})", ldap3::ldap_escape(upn)),
    };
    format!(
        "(&(objectCategory=person)(objectClass=user){name}{})",
        extra.unwrap_or_default()
    )
}

/// Maps the Active Directory sub-code in a failed bind's diagnostic message
/// ("… AcceptSecurityContext error, data 52e, v4563") to a reason.
fn bind_failure(diagnostic: &str) -> AuthError {
    let code = diagnostic
        .split("data ")
        .nth(1)
        .and_then(|rest| rest.split([',', ' ']).next())
        .map(str::to_ascii_lowercase);
    match code.as_deref() {
        Some("530" | "531" | "533") => AuthError::AccountDisabled,
        Some("532") => AuthError::PasswordExpired,
        Some("701") => AuthError::AccountExpired,
        Some("773") => AuthError::PasswordMustChange,
        Some("775") => AuthError::AccountLocked,
        _ => AuthError::InvalidCredentials,
    }
}

fn tls_config(ca_file: &Path) -> Result<Arc<ClientConfig>, String> {
    let describe = |e: &dyn std::fmt::Display| format!("CA file {}: {e}", ca_file.display());
    let mut roots = RootCertStore::empty();
    for cert in CertificateDer::pem_file_iter(ca_file).map_err(|e| describe(&e))? {
        roots
            .add(cert.map_err(|e| describe(&e))?)
            .map_err(|e| describe(&e))?;
    }
    if roots.is_empty() {
        return Err(describe(&"no certificates found"));
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| describe(&e))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn unavailable(error: LdapError) -> AuthError {
    AuthError::Unavailable(error.to_string())
}

fn directory(error: LdapError) -> AuthError {
    AuthError::Directory(error.to_string())
}

/// Attribute names from the server may differ in case from what we asked for.
fn lookup<'a, T>(map: &'a HashMap<String, Vec<T>>, name: &str) -> Option<&'a Vec<T>> {
    map.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, values)| values)
}

/// Binary values of an attribute. ldap3 files values that happen to be valid
/// UTF-8 under text attributes, so both maps are consulted.
fn binary_values(entry: &SearchEntry, name: &str) -> Vec<Vec<u8>> {
    let mut values: Vec<Vec<u8>> = lookup(&entry.bin_attrs, name).cloned().unwrap_or_default();
    if let Some(text) = lookup(&entry.attrs, name) {
        values.extend(text.iter().map(|s| s.as_bytes().to_vec()));
    }
    values
}

fn first_binary(entry: &SearchEntry, name: &str) -> Option<Vec<u8>> {
    binary_values(entry, name).into_iter().next()
}

fn first_text(entry: &SearchEntry, name: &str) -> Option<String> {
    lookup(&entry.attrs, name)?
        .first()
        .filter(|value| !value.is_empty())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_name_forms_users_type() {
        assert_eq!(Account::parse("alice"), Some(Account::Sam("alice".into())));
        assert_eq!(
            Account::parse(" EXAMPLE\\alice "),
            Some(Account::Sam("alice".into()))
        );
        assert_eq!(
            Account::parse("alice@example.com"),
            Some(Account::Upn("alice@example.com".into()))
        );
        for invalid in ["", "   ", "EXAMPLE\\", "a\u{0}b", &"x".repeat(257)] {
            assert_eq!(Account::parse(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn escapes_names_in_the_filter() {
        assert_eq!(
            user_filter(&Account::Sam("*)(cn=*".into()), None),
            r"(&(objectCategory=person)(objectClass=user)(sAMAccountName=\2a\29\28cn=\2a))"
        );
        assert_eq!(
            user_filter(&Account::Upn("a@b".into()), Some("(memberOf=CN=x)")),
            "(&(objectCategory=person)(objectClass=user)(userPrincipalName=a@b)(memberOf=CN=x))"
        );
    }

    #[test]
    fn maps_active_directory_bind_codes() {
        let diagnostic = |code: &str| {
            format!(
                "80090308: LdapErr: DSID-0C0903A9, comment: AcceptSecurityContext error, data {code}, v1db1"
            )
        };
        assert_eq!(
            bind_failure(&diagnostic("52e")),
            AuthError::InvalidCredentials
        );
        assert_eq!(
            bind_failure(&diagnostic("525")),
            AuthError::InvalidCredentials
        );
        assert_eq!(bind_failure(&diagnostic("533")), AuthError::AccountDisabled);
        assert_eq!(bind_failure(&diagnostic("532")), AuthError::PasswordExpired);
        assert_eq!(bind_failure(&diagnostic("701")), AuthError::AccountExpired);
        assert_eq!(
            bind_failure(&diagnostic("773")),
            AuthError::PasswordMustChange
        );
        assert_eq!(bind_failure(&diagnostic("775")), AuthError::AccountLocked);
        assert_eq!(
            bind_failure("something else"),
            AuthError::InvalidCredentials
        );
    }

    #[test]
    fn reports_unusable_ca_files() {
        assert!(tls_config(Path::new("/does/not/exist.pem")).is_err());
        let empty = std::env::temp_dir().join("remotehub-empty-ca.pem");
        std::fs::write(&empty, "").unwrap();
        assert!(tls_config(&empty).unwrap_err().contains("no certificates"));
    }
}
