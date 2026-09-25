//! The audit log: who did what, when, from where (migration `audit_log`).
//!
//! Entries are only ever appended; the database numbers and hash-chains them
//! (trigger `audit_chain`). [`verify`] recomputes the chain and reports the
//! first entry that does not match.

use serde::Serialize;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

/// Defines the audit actions once: variant and stable wire name. The UI shows
/// the message `audit_<name with dots as underscores>`; generated TypeScript
/// and a test make sure every action has one.
macro_rules! audit_actions {
    ($($variant:ident = $name:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        pub enum Action {
            $(#[serde(rename = $name)] $variant,)+
        }

        impl Action {
            pub const ALL: &[Action] = &[$(Action::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Action::$variant => $name,)+ }
            }
        }
    };
}

audit_actions! {
    SignIn = "session.sign_in",
    SignInFailed = "session.sign_in_failed",
    SignOut = "session.sign_out",
    AuditVerified = "audit.verified",
    BreakGlassCreated = "break_glass.created",
    BreakGlassReset = "break_glass.reset",
    BreakGlassDeleted = "break_glass.deleted",
    SetupCompleted = "setup.completed",
    DirectoryChanged = "directory.changed",
    DirectoryRemoved = "directory.removed",
    FolderCreated = "folder.created",
    FolderUpdated = "folder.updated",
    FolderDeleted = "folder.deleted",
    DeviceCreated = "device.created",
    DeviceUpdated = "device.updated",
    DeviceDeleted = "device.deleted",
    CredentialCreated = "credential.created",
    CredentialUpdated = "credential.updated",
    CredentialDeleted = "credential.deleted",
    GrantAdded = "grant.added",
    GrantRemoved = "grant.removed",
    ConnectionOpened = "connection.opened",
    ConnectionClosed = "connection.closed",
    ConnectionFailed = "connection.failed",
    HostKeyPinned = "device.host_key_pinned",
    HostKeyReset = "device.host_key_reset",
    CertificatePinned = "device.certificate_pinned",
    CertificateReset = "device.certificate_reset",
    AccessRequested = "access.requested",
    AccessApproved = "access.approved",
    AccessDenied = "access.denied",
    AccessCancelled = "access.cancelled",
    PersonalUnlockAdded = "personal.unlock_added",
    PersonalUnlockRemoved = "personal.unlock_removed",
    PersonalEntrySaved = "personal.entry_saved",
    PersonalEntryDeleted = "personal.entry_deleted",
    PersonalVaultReset = "personal.vault_reset",
    ConnectorCreated = "connector.created",
    ConnectorDeleted = "connector.deleted",
    PurposeRequired = "purpose.required",
    PurposeWaived = "purpose.waived",
    AccountInvited = "account.invited",
    AccountRecoveryIssued = "account.recovery_issued",
    AccountDeleted = "account.deleted",
    UserBlocked = "user.blocked",
    UserUnblocked = "user.unblocked",
    UserSessionsEnded = "user.sessions_ended",
    GroupCreated = "group.created",
    GroupUpdated = "group.updated",
    GroupDeleted = "group.deleted",
    GroupMemberAdded = "group.member_added",
    GroupMemberRemoved = "group.member_removed",
    RoleAssigned = "role.assigned",
    RoleRevoked = "role.revoked",
    SessionsEndedByDirectory = "session.ended_by_directory",
    SecondFactorEnrolled = "second_factor.enrolled",
    SecondFactorRemoved = "second_factor.removed",
    SecondFactorReset = "second_factor.reset",
    SecondFactorRequired = "second_factor.required",
    SecondFactorWaived = "second_factor.waived",
    CredentialRevealed = "credential.revealed",
    CredentialAttachmentAdded = "credential.attachment_added",
    CredentialAttachmentDeleted = "credential.attachment_deleted",
    PersonalAttachmentSaved = "personal.attachment_saved",
    PersonalAttachmentDeleted = "personal.attachment_deleted",
    RecoveryKeyCreated = "recovery_key.created",
    RecoveryKeyDeleted = "recovery_key.deleted",
    VaultRecoveryRequested = "vault_recovery.requested",
    VaultRecoveryApproved = "vault_recovery.approved",
    VaultRecoveryCancelled = "vault_recovery.cancelled",
    VaultRecoveryOpened = "vault_recovery.opened",
    VaultRecoveryCompleted = "vault_recovery.completed",
}

/// Who acted: a signed-in user, or only a name (failed sign-in).
#[derive(Debug, Clone, Copy)]
pub struct Actor<'a> {
    pub id: Option<Uuid>,
    pub name: &'a str,
}

#[derive(Debug)]
pub struct Entry<'a> {
    pub actor: Actor<'a>,
    pub action: Action,
    /// The object acted on, e.g. `("credential", id)`.
    pub object: Option<(&'a str, Uuid)>,
    /// Additional facts; never secrets.
    pub details: serde_json::Value,
    pub address: Option<&'a str>,
}

/// Appends an entry. Pass the transaction of the action itself, so both are
/// committed together or not at all.
pub async fn record<'e>(db: impl PgExecutor<'e>, entry: Entry<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_log (actor_id, actor_name, action, object_type, object_id, details, address)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(entry.actor.id)
    .bind(entry.actor.name)
    .bind(entry.action.as_str())
    .bind(entry.object.map(|(kind, _)| kind))
    .bind(entry.object.map(|(_, id)| id))
    .bind(sqlx::types::Json(&entry.details))
    .bind(entry.address)
    .execute(db)
    .await?;
    Ok(())
}

/// An entry as shown in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct Record {
    pub seq: i64,
    /// RFC 3339 in UTC; the UI formats it in the viewer's locale.
    pub at: String,
    pub actor_name: String,
    pub action: String,
    pub object_type: Option<String>,
    pub object_id: Option<Uuid>,
    pub details: sqlx::types::Json<serde_json::Value>,
    pub address: Option<String>,
}

/// The newest entries, optionally only those older than `before`.
pub async fn list(
    db: &PgPool,
    before: Option<i64>,
    limit: i64,
) -> Result<Vec<Record>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT seq,
                  to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS at,
                  actor_name, action, object_type, object_id, details, address
           FROM audit_log
           WHERE $1::bigint IS NULL OR seq < $1
           ORDER BY seq DESC
           LIMIT $2"#,
    )
    .bind(before)
    .bind(limit.clamp(1, 500))
    .fetch_all(db)
    .await
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verification {
    pub entries: i64,
    /// The first entry whose hash, link to its predecessor or number does not
    /// match; everything from there on is untrustworthy.
    pub first_broken: Option<i64>,
}

/// Recomputes the whole chain.
pub async fn verify(db: &PgPool) -> Result<Verification, sqlx::Error> {
    let (entries, first_broken): (i64, Option<i64>) = sqlx::query_as(
        "WITH checked AS (
             SELECT seq,
                    hash = audit_digest(prev_hash, seq, at, actor_id, actor_name, action,
                                        object_type, object_id, details, address)
                    AND prev_hash = coalesce(lag(hash) OVER (ORDER BY seq), audit_genesis())
                    AND seq = row_number() OVER (ORDER BY seq) AS ok
             FROM audit_log
         )
         SELECT count(*), min(seq) FILTER (WHERE NOT ok) FROM checked",
    )
    .fetch_one(db)
    .await?;
    Ok(Verification {
        entries,
        first_broken,
    })
}

/// TypeScript for the UI: the list of actions. Checked in at
/// `web/src/lib/api/generated/audit.ts`; a test fails when it is stale.
pub fn typescript() -> String {
    let actions = Action::ALL
        .iter()
        .map(|a| format!("\t'{}'", a.as_str()))
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "// Generated from crates/server/src/audit.rs — do not edit.\n\
         // Regenerate: REMOTEHUB_BLESS=1 cargo nextest run -p remotehub-server generated\n\
         \n\
         export const AUDIT_ACTIONS = [\n{actions}\n] as const;\n\
         \n\
         export type AuditAction = (typeof AUDIT_ACTIONS)[number];\n"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn action_names_are_unique_and_dotted_snake_case() {
        let mut seen = HashSet::new();
        for action in Action::ALL {
            let name = action.as_str();
            assert!(seen.insert(name), "duplicate {name}");
            assert!(
                name.contains('.')
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'),
                "{name}"
            );
            assert_eq!(serde_json::to_value(action).unwrap(), name);
        }
    }
}
