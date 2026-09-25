//! Sign-in against directories (ADR 0005).
//!
//! [`IdentityProvider`] checks a user's credentials and returns their
//! [`Identity`]: immutable IDs (SIDs, GUID) and all group memberships,
//! including nested ones. The first implementation is [`ldap::LdapDirectory`]
//! for Active Directory; Entra ID (OIDC) follows in M4.

pub mod check;
pub mod laps;
pub mod ldap;
mod sid;
mod trust;

use std::future::Future;

use secrecy::SecretString;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub use sid::{InvalidSid, Sid};

/// Who signed in. Permissions refer to `sid` and `groups`, never to names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Identity {
    pub sid: Sid,
    /// `objectGUID`: stays the same across renames and moves.
    pub guid: Uuid,
    /// `sAMAccountName`, e.g. `alice`.
    pub username: String,
    /// `userPrincipalName`, e.g. `alice@example.com`.
    pub upn: Option<String>,
    pub display_name: String,
    pub email: Option<String>,
    /// SIDs of every security group the user is in, directly or nested.
    pub groups: Vec<Sid>,
}

/// A user or group found in the directory, for choosing whom to grant access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Principal {
    pub kind: PrincipalKind,
    pub sid: Sid,
    pub name: String,
    /// For users: the UPN or account name, to tell people apart.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Group,
    User,
}

/// A group found in the directory, e.g. for choosing whom to grant access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub sid: Sid,
    pub name: String,
}

/// Why a sign-in failed. The reasons are for the audit log; the API decides
/// how much of it a client learns.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    /// Unknown user, wrong or empty password.
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("account disabled")]
    AccountDisabled,
    #[error("account locked out")]
    AccountLocked,
    #[error("account expired")]
    AccountExpired,
    #[error("password expired")]
    PasswordExpired,
    #[error("password must be changed")]
    PasswordMustChange,
    /// The directory could not be reached (network, TLS, timeout).
    #[error("directory unavailable: {0}")]
    Unavailable(String),
    /// The directory answered unexpectedly (configuration, permissions).
    #[error("directory error: {0}")]
    Directory(String),
}

/// A source of identities that can verify a password.
pub trait IdentityProvider: Send + Sync {
    fn authenticate(
        &self,
        username: &str,
        password: &SecretString,
    ) -> impl Future<Output = Result<Identity, AuthError>> + Send;

    /// Users and groups whose name contains `query` (at least two characters).
    fn search(
        &self,
        query: &str,
        limit: i32,
    ) -> impl Future<Output = Result<Vec<Principal>, AuthError>> + Send;

    /// The groups of the user with `sid` now (#108), read without their
    /// password. Fails with `AccountDisabled` or `AccountExpired` if the
    /// account may no longer sign in, and with `InvalidCredentials` if it is
    /// gone or no longer matches the user filter.
    fn refresh(&self, sid: &Sid) -> impl Future<Output = Result<Vec<Sid>, AuthError>> + Send;

    /// The LAPS password of the computer that `host` names, read now.
    fn laps_password(
        &self,
        host: &str,
    ) -> impl Future<Output = Result<laps::LapsPassword, laps::LapsError>> + Send;
}
