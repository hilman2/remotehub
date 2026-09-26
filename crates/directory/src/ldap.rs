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
use std::sync::Arc;
use std::time::Duration;

use ldap3::{
    Ldap, LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry, SearchOptions,
    SearchResult,
};
use rustls::ClientConfig;
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use crate::check::{Failure, Reason, Step};
use crate::laps::{self, LapsError, LapsPassword};
use crate::trust::{self, Verifier};
use crate::{AuthError, Group, Identity, IdentityProvider, Principal, PrincipalKind, Sid};

/// LDAP result codes.
const SIZE_LIMIT_EXCEEDED: u32 = 4;
const NO_SUCH_OBJECT: u32 = 32;
const INVALID_CREDENTIALS: u32 = 49;

/// `ACCOUNTDISABLE` in `userAccountControl`.
const ACCOUNT_DISABLED: u32 = 0x2;
/// Seconds from 1601-01-01, where Windows file times start, to 1970-01-01.
const FILETIME_TO_UNIX: u64 = 11_644_473_600;

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
    /// PEM with the CA certificates that sign the domain controllers'
    /// certificates; without it the system's roots are used.
    pub ca_pem: Option<String>,
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
    tls: Arc<ClientConfig>,
}

impl LdapDirectory {
    pub fn new(config: LdapConfig) -> Result<Self, AuthError> {
        let tls = Verifier::new(config.ca_pem.as_deref())
            .and_then(trust::client_config)
            .map_err(AuthError::Directory)?;
        Ok(LdapDirectory { config, tls })
    }

    async fn connect(&self) -> Result<Ldap, AuthError> {
        let settings = LdapConnSettings::new()
            .set_conn_timeout(self.config.timeout)
            .set_starttls(self.config.starttls)
            .set_config(self.tls.clone());
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

    /// The last steps of [`crate::check::check`] (#144): binds as the
    /// service account, then asks for the users the filter lets sign in, up
    /// to `limit`. Returns their names, and whether there are more.
    pub(crate) async fn users(&self, limit: i32) -> Result<(Vec<String>, bool), Failure> {
        let mut ldap = self
            .connect()
            .await
            .map_err(|e| Failure::new(Step::Connect, Reason::Other, e))?;
        let bound = ldap
            .with_timeout(self.config.timeout)
            .simple_bind(
                &self.config.bind_dn,
                self.config.bind_password.expose_secret(),
            )
            .await
            .map_err(|e| Failure::new(Step::Bind, Reason::Other, e))?;
        match bound.rc {
            0 => {}
            INVALID_CREDENTIALS => {
                return Err(Failure::new(
                    Step::Bind,
                    Reason::InvalidCredentials,
                    bound.text,
                ));
            }
            rc => {
                let detail = format!("code {rc}: {}", bound.text);
                return Err(Failure::new(Step::Bind, Reason::Other, detail));
            }
        }
        let filter = format!(
            "(&(objectCategory=person)(objectClass=user){})",
            self.config.user_filter.as_deref().unwrap_or_default()
        );
        let SearchResult(entries, result) = ldap
            .with_timeout(self.config.timeout)
            .with_search_options(SearchOptions::new().sizelimit(limit))
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                vec!["displayName", "sAMAccountName"],
            )
            .await
            .map_err(|e| Failure::new(Step::Search, Reason::Other, e))?;
        let _ = ldap.unbind().await;
        match result.rc {
            0 | SIZE_LIMIT_EXCEEDED => {}
            NO_SUCH_OBJECT => {
                return Err(Failure::new(Step::Search, Reason::NoSuchBase, result.text));
            }
            rc => {
                let detail = format!("code {rc}: {}", result.text);
                return Err(Failure::new(Step::Search, Reason::Other, detail));
            }
        }
        let names = entries
            .into_iter()
            .filter_map(|entry| {
                let entry = SearchEntry::construct(entry);
                first_text(&entry, "displayName").or_else(|| first_text(&entry, "sAMAccountName"))
            })
            .collect();
        Ok((names, result.rc == SIZE_LIMIT_EXCEEDED))
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

    /// Users whose account name, display name or UPN contains `query`.
    pub async fn search_users(&self, query: &str, limit: i32) -> Result<Vec<Principal>, AuthError> {
        let query = ldap3::ldap_escape(query.trim());
        let filter = format!(
            "(&(objectCategory=person)(objectClass=user)(|(sAMAccountName=*{query}*)(displayName=*{query}*)(userPrincipalName=*{query}*)){})",
            self.config.user_filter.as_deref().unwrap_or_default()
        );
        let mut ldap = self.service().await?;
        let SearchResult(entries, result) = ldap
            .with_timeout(self.config.timeout)
            .with_search_options(SearchOptions::new().sizelimit(limit))
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                vec![
                    "sAMAccountName",
                    "displayName",
                    "userPrincipalName",
                    "objectSid",
                ],
            )
            .await
            .map_err(unavailable)?;
        let _ = ldap.unbind().await;
        if result.rc != 0 && result.rc != SIZE_LIMIT_EXCEEDED {
            return Err(AuthError::Directory(format!(
                "user search failed with code {}: {}",
                result.rc, result.text
            )));
        }
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let entry = SearchEntry::construct(entry);
                let account = first_text(&entry, "sAMAccountName")?;
                Some(Principal {
                    kind: PrincipalKind::User,
                    sid: Sid::from_bytes(first_binary(&entry, "objectSid")?.as_slice()).ok()?,
                    name: first_text(&entry, "displayName").unwrap_or_else(|| account.clone()),
                    detail: first_text(&entry, "userPrincipalName").or(Some(account)),
                })
            })
            .collect())
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
    async fn laps_password(&self, host: &str) -> Result<LapsPassword, LapsError> {
        let filter = laps::filter(host).ok_or_else(|| LapsError::ComputerNotFound(host.into()))?;
        let mut ldap = self.service().await?;
        let SearchResult(entries, result) = ldap
            .with_timeout(self.config.timeout)
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                laps::ATTRIBUTES.to_vec(),
            )
            .await
            .map_err(unavailable)?;
        let _ = ldap.unbind().await;
        if result.rc != 0 {
            return Err(AuthError::Directory(format!(
                "computer search failed with code {}: {}",
                result.rc, result.text
            ))
            .into());
        }
        let entries: Vec<SearchEntry> = entries.into_iter().map(SearchEntry::construct).collect();
        let dns_names: Vec<Option<String>> = entries
            .iter()
            .map(|entry| first_text(entry, "dNSHostName"))
            .collect();
        let entry = &entries[laps::choose(&dns_names, host)?];
        let computer = first_text(entry, "sAMAccountName").unwrap_or_default();
        laps::password(
            first_text(entry, "msLAPS-Password").as_deref(),
            first_text(entry, "ms-Mcs-AdmPwd").as_deref(),
            &computer,
        )
        .ok_or_else(|| LapsError::NoPassword(host.into()))
    }

    async fn refresh(&self, sid: &Sid) -> Result<Vec<Sid>, AuthError> {
        let filter = format!(
            "(&(objectCategory=person)(objectClass=user)(objectSid={}){})",
            escape_binary(&sid.to_bytes()),
            self.config.user_filter.as_deref().unwrap_or_default()
        );
        let mut service = self.service().await?;
        let (entries, _) = service
            .with_timeout(self.config.timeout)
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                vec!["userAccountControl", "accountExpires"],
            )
            .await
            .map_err(unavailable)?
            .success()
            .map_err(directory)?;
        let Some(entry) = entries.into_iter().next().map(SearchEntry::construct) else {
            let _ = service.unbind().await;
            return Err(AuthError::InvalidCredentials);
        };
        let control: u32 = first_text(&entry, "userAccountControl")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if control & ACCOUNT_DISABLED != 0 {
            let _ = service.unbind().await;
            return Err(AuthError::AccountDisabled);
        }
        let expires = first_text(&entry, "accountExpires").and_then(|v| v.parse().ok());
        if expired(expires, std::time::SystemTime::now()) {
            let _ = service.unbind().await;
            return Err(AuthError::AccountExpired);
        }
        let groups = self.token_groups(&mut service, &entry.dn).await;
        let _ = service.unbind().await;
        groups
    }

    async fn group_names(&self, sids: &[Sid]) -> Result<Vec<Group>, AuthError> {
        let mut found = Vec::new();
        // A bounded filter per search; a user rarely has more groups.
        for chunk in sids.chunks(100) {
            let any: String = chunk
                .iter()
                .map(|sid| format!("(objectSid={})", escape_binary(&sid.to_bytes())))
                .collect();
            let filter = format!("(&(objectCategory=group)(|{any}))");
            let mut ldap = self.service().await?;
            let SearchResult(entries, result) = ldap
                .with_timeout(self.config.timeout)
                .search(
                    &self.config.base_dn,
                    Scope::Subtree,
                    &filter,
                    vec!["cn", "objectSid"],
                )
                .await
                .map_err(unavailable)?;
            let _ = ldap.unbind().await;
            if result.rc != 0 {
                return Err(AuthError::Directory(format!(
                    "group lookup failed with code {}: {}",
                    result.rc, result.text
                )));
            }
            found.extend(entries.into_iter().filter_map(|entry| {
                let entry = SearchEntry::construct(entry);
                Some(Group {
                    sid: Sid::from_bytes(first_binary(&entry, "objectSid")?.as_slice()).ok()?,
                    name: first_text(&entry, "cn")?,
                })
            }));
        }
        Ok(found)
    }

    async fn search(&self, query: &str, limit: i32) -> Result<Vec<Principal>, AuthError> {
        if query.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        let mut found: Vec<Principal> = self
            .search_groups(query, limit)
            .await?
            .into_iter()
            .map(|group| Principal {
                kind: PrincipalKind::Group,
                sid: group.sid,
                name: group.name,
                detail: None,
            })
            .collect();
        found.extend(self.search_users(query, limit).await?);
        found.sort_by_key(|p| (p.kind, p.name.to_lowercase()));
        Ok(found)
    }

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

/// A filter value that matches `bytes` exactly: every byte as `\hh`.
fn escape_binary(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// Whether `accountExpires` (100-nanosecond steps since 1601) lies before
/// `now`. 0 means never; so does `i64::MAX`, which lies 29 000 years ahead.
fn expired(account_expires: Option<i64>, now: std::time::SystemTime) -> bool {
    let Some(expires) = account_expires.filter(|&e| e > 0) else {
        return false;
    };
    let now = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() + FILETIME_TO_UNIX);
    (expires as u64) / 10_000_000 < now
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
    fn escapes_every_byte_of_a_binary_value() {
        assert_eq!(escape_binary(&[1, 0x2a, 0xff]), r"\01\2a\ff");
    }

    #[test]
    fn reads_when_an_account_expires() {
        use std::time::{Duration, UNIX_EPOCH};
        // 2026-01-01T00:00:00Z as a file time.
        let new_year = (1_767_225_600 + FILETIME_TO_UNIX as i64) * 10_000_000;
        let at = |secs: u64| UNIX_EPOCH + Duration::from_secs(secs);
        assert!(!expired(Some(new_year), at(1_767_225_599)));
        assert!(expired(Some(new_year), at(1_767_225_601)));
        for never in [None, Some(0), Some(i64::MAX)] {
            assert!(!expired(never, at(4_000_000_000)), "{never:?}");
        }
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
}
