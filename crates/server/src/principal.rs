//! Whom grants, purpose rules and group memberships name: a directory user
//! or group by SID, a local account by its Kratos identity, or a group of
//! remotehub's own (#105). Stored as text: `S-1-5-…`, `local:<uuid>`,
//! `group:<uuid>`.

use std::fmt;
use std::str::FromStr;

use remotehub_directory::Sid;
use serde::Serialize;
use sqlx::PgExecutor;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrincipalId {
    Directory(Sid),
    Local(Uuid),
    Group(Uuid),
}

impl FromStr for PrincipalId {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        if let Some(id) = s.strip_prefix("local:") {
            id.parse().map(PrincipalId::Local).map_err(|_| ())
        } else if let Some(id) = s.strip_prefix("group:") {
            id.parse().map(PrincipalId::Group).map_err(|_| ())
        } else {
            s.parse().map(PrincipalId::Directory).map_err(|_| ())
        }
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrincipalId::Directory(sid) => f.write_str(sid.as_str()),
            PrincipalId::Local(id) => write!(f, "local:{id}"),
            PrincipalId::Group(id) => write!(f, "group:{id}"),
        }
    }
}

impl PrincipalId {
    /// Whether `kind` (`user` or `group`) fits and, for principals remotehub
    /// keeps itself, whether they exist. A directory SID is taken as given:
    /// the directory is not asked at every grant.
    pub async fn check<'e>(&self, db: impl PgExecutor<'e>, kind: &str) -> sqlx::Result<bool> {
        match self {
            PrincipalId::Directory(_) => Ok(matches!(kind, "user" | "group")),
            PrincipalId::Local(id) if kind == "user" => {
                sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM users WHERE identity_id = $1 AND kind = 'local')",
                )
                .bind(id)
                .fetch_one(db)
                .await
            }
            PrincipalId::Group(id) if kind == "group" => {
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1)")
                    .bind(id)
                    .fetch_one(db)
                    .await
            }
            _ => Ok(false),
        }
    }
}

/// A user or group to choose, e.g. for a grant. Serialised like the
/// directory's principals, so the UI treats all alike.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Principal {
    /// `user` or `group`.
    pub kind: &'static str,
    pub sid: String,
    pub name: String,
    /// The account name or e-mail address of a user, the description of a
    /// group of remotehub's own.
    pub detail: Option<String>,
}

impl From<remotehub_directory::Principal> for Principal {
    fn from(found: remotehub_directory::Principal) -> Self {
        Principal {
            kind: match found.kind {
                remotehub_directory::PrincipalKind::User => "user",
                remotehub_directory::PrincipalKind::Group => "group",
            },
            sid: found.sid.to_string(),
            name: found.name,
            detail: found.detail,
        }
    }
}

/// Groups of remotehub's own and local accounts whose name contains `query`.
pub async fn search_own(
    db: &sqlx::PgPool,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<Principal>> {
    let pattern = format!(
        "%{}%",
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "(SELECT 'group', 'group:' || id, name, nullif(description, '') FROM groups
          WHERE name ILIKE $1 ORDER BY lower(name) LIMIT $2)
         UNION ALL
         (SELECT 'user', 'local:' || identity_id, display_name, username FROM users
          WHERE kind = 'local' AND (display_name ILIKE $1 OR username ILIKE $1)
          ORDER BY lower(display_name) LIMIT $2)",
    )
    .bind(pattern)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(kind, sid, name, detail)| Principal {
            kind: if kind == "group" { "group" } else { "user" },
            sid,
            name,
            detail,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principals_read_back_as_written() {
        for text in [
            "S-1-5-21-1-2-3-1105",
            "local:5b0c3cb1-7a0e-4a5e-9d0a-0000000000a1",
            "group:5b0c3cb1-7a0e-4a5e-9d0a-0000000000a2",
        ] {
            let id: PrincipalId = text.parse().unwrap();
            assert_eq!(id.to_string(), text);
        }
        for wrong in ["", "alice", "local:alice", "group:", "S-1-x"] {
            assert!(wrong.parse::<PrincipalId>().is_err(), "{wrong}");
        }
    }
}
