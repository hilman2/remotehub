//! The Access page (#178): who signs in, which groups and roles they hold,
//! and which grants a user or group has, for administrators and auditors.
//! Changes go through the endpoints that exist for them (users, groups,
//! roles, grants); this module only reads.
//!
//! - `GET /api/access`: users with their groups and roles, and every group
//!   remotehub knows with its members
//! - `GET /api/access/grants?principal=SID`: the grants of one user or group

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use super::reports::{Names, principal_names};
use crate::AppState;
use crate::session::Session;

fn require_auditor(session: &Session) -> Result<(), Problem> {
    if session.is_auditor() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    kind: String,
    username: String,
    display_name: String,
    email: Option<String>,
    last_sign_in_at: Option<String>,
    blocked: bool,
    second_factor: bool,
    sessions: i64,
    sid: Option<String>,
    identity_id: Option<Uuid>,
    /// The directory groups of the latest sign-in.
    groups: Option<Vec<String>>,
}

/// A user or group as a grant or membership names it.
#[derive(Serialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Named {
    sid: String,
    name: String,
}

#[derive(Serialize)]
pub struct UserGroup {
    sid: String,
    name: String,
    /// `directory` or `own` (remotehub's own groups).
    source: &'static str,
    /// For an own group: the directory group through which the user is in
    /// it; none if they are a member themselves.
    via: Option<Named>,
}

#[derive(Serialize)]
pub struct UserRole {
    role: String,
    /// The user or group the role is assigned to.
    via: Named,
}

#[derive(Serialize)]
pub struct User {
    id: Uuid,
    kind: String,
    username: String,
    display_name: String,
    email: Option<String>,
    last_sign_in_at: Option<String>,
    blocked: bool,
    second_factor: bool,
    sessions: i64,
    /// Named like the principal of a grant; none for a break-glass account.
    principal: Option<String>,
    groups: Vec<UserGroup>,
    roles: Vec<UserRole>,
}

#[derive(Serialize)]
pub struct Member {
    sid: String,
    name: String,
    /// `user` or `group`.
    kind: String,
    /// The remotehub user, if the member is one.
    user_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct Group {
    sid: String,
    name: String,
    /// `directory` or `own`.
    source: &'static str,
    /// An own group's id and description.
    id: Option<Uuid>,
    description: String,
    /// An own group's members as added; a directory group's users as of
    /// their latest sign-in.
    members: Vec<Member>,
    /// The own groups it is a member of.
    member_of: Vec<Named>,
    roles: Vec<String>,
    /// Grants to it that have not ended.
    grants: i64,
}

#[derive(Serialize)]
pub struct Overview {
    users: Vec<User>,
    groups: Vec<Group>,
}

pub async fn overview(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Overview>, Problem> {
    require_auditor(&session)?;
    let users: Vec<UserRow> = sqlx::query_as(
        "SELECT u.id, u.kind, u.username, u.display_name, u.email,
                to_char(u.last_sign_in_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                    AS last_sign_in_at,
                u.blocked_at IS NOT NULL AS blocked,
                (u.kind <> 'directory'
                 OR EXISTS (SELECT 1 FROM second_factors f WHERE f.user_id = u.id)
                 OR EXISTS (SELECT 1 FROM security_keys k WHERE k.user_id = u.id))
                    AS second_factor,
                (SELECT count(*) FROM sessions s WHERE s.user_id = u.id AND s.expires_at > now())
                    AS sessions,
                u.sid, u.identity_id,
                (SELECT s.groups FROM sessions s WHERE s.user_id = u.id
                 ORDER BY s.created_at DESC LIMIT 1) AS groups
         FROM users u WHERE u.kind <> 'deleted'
         ORDER BY lower(u.display_name), u.id",
    )
    .fetch_all(&state.db)
    .await?;
    let own: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id, name, description FROM groups ORDER BY lower(name)")
            .fetch_all(&state.db)
            .await?;
    let members: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT group_id, principal_sid, principal_kind, principal_name FROM group_members
         ORDER BY lower(principal_name)",
    )
    .fetch_all(&state.db)
    .await?;
    let assignments: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT role, principal_sid, principal_kind, principal_name FROM role_assignments",
    )
    .fetch_all(&state.db)
    .await?;
    let grants: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT principal_sid, count(*) FROM grants
         WHERE expires_at IS NULL OR expires_at > now() GROUP BY principal_sid",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let directory_group_sids: Vec<String> = sqlx::query_scalar(
        "SELECT principal_sid FROM grants WHERE principal_kind = 'group'
                                            AND principal_sid NOT LIKE 'group:%'
         UNION SELECT principal_sid FROM group_members WHERE principal_kind = 'group'
         UNION SELECT principal_sid FROM role_assignments WHERE principal_kind = 'group'
                                                        AND principal_sid NOT LIKE 'group:%'",
    )
    .fetch_all(&state.db)
    .await?;
    let names = principal_names(&state).await?;
    let label = |sid: &str| names.get(sid).cloned().unwrap_or_else(|| sid.to_owned());
    let named = |sid: &str| Named {
        sid: sid.to_owned(),
        name: label(sid),
    };
    let own_sid = |id: Uuid| format!("group:{id}");

    // Whom each principal counts as a member of: own groups by their members.
    let mut in_own: HashMap<&str, Vec<Uuid>> = HashMap::new();
    for (group, sid, _, _) in &members {
        in_own.entry(sid.as_str()).or_default().push(*group);
    }
    let own_name: HashMap<Uuid, &str> = own.iter().map(|(id, n, _)| (*id, n.as_str())).collect();
    let mut roles_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for (role, sid, _, _) in &assignments {
        roles_of
            .entry(sid.as_str())
            .or_default()
            .push(role.as_str());
    }

    let mut directory_members: BTreeMap<String, Vec<Member>> = directory_group_sids
        .into_iter()
        .map(|sid| (sid, Vec::new()))
        .collect();
    let users = users
        .into_iter()
        .map(|row| {
            let principal = match (row.kind.as_str(), &row.sid, row.identity_id) {
                ("directory", Some(sid), _) => Some(sid.clone()),
                (_, _, Some(identity)) => Some(format!("local:{identity}")),
                _ => None,
            };
            let directory: Vec<String> = row.groups.unwrap_or_default();
            let mut groups: Vec<UserGroup> = Vec::new();
            for sid in &directory {
                directory_members
                    .entry(sid.clone())
                    .or_default()
                    .push(Member {
                        sid: principal.clone().unwrap_or_default(),
                        name: row.display_name.clone(),
                        kind: "user".into(),
                        user_id: Some(row.id),
                    });
                groups.push(UserGroup {
                    sid: sid.clone(),
                    name: label(sid),
                    source: "directory",
                    via: None,
                });
            }
            // Own groups, as `session::lookup` counts them: the user's own
            // SID or one of their directory groups is a member.
            let mut seen = BTreeSet::new();
            let holders = principal
                .iter()
                .map(|p| (p, None))
                .chain(directory.iter().map(|sid| (sid, Some(named(sid)))));
            for (sid, via) in holders {
                for group in in_own.get(sid.as_str()).into_iter().flatten() {
                    if seen.insert(*group) {
                        groups.push(UserGroup {
                            sid: own_sid(*group),
                            name: own_name.get(group).copied().unwrap_or_default().to_owned(),
                            source: "own",
                            via: via.clone(),
                        });
                    }
                }
            }
            let sids: Vec<String> = principal
                .iter()
                .cloned()
                .chain(groups.iter().map(|g| g.sid.clone()))
                .collect();
            let roles = sids
                .iter()
                .flat_map(|sid| {
                    roles_of
                        .get(sid.as_str())
                        .into_iter()
                        .flatten()
                        .map(|role| UserRole {
                            role: (*role).to_owned(),
                            via: named(sid),
                        })
                })
                .collect();
            User {
                id: row.id,
                kind: row.kind,
                username: row.username,
                display_name: row.display_name,
                email: row.email,
                last_sign_in_at: row.last_sign_in_at,
                blocked: row.blocked,
                second_factor: row.second_factor,
                sessions: row.sessions,
                principal,
                groups,
                roles,
            }
        })
        .collect();

    let user_ids: HashMap<String, Uuid> = sqlx::query_as::<_, (String, Uuid)>(
        "SELECT sid, id FROM users WHERE sid IS NOT NULL
         UNION ALL SELECT 'local:' || identity_id, id FROM users WHERE identity_id IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let member_of = |sid: &str| -> Vec<Named> {
        in_own
            .get(sid)
            .into_iter()
            .flatten()
            .map(|group| Named {
                sid: own_sid(*group),
                name: own_name.get(group).copied().unwrap_or_default().to_owned(),
            })
            .collect()
    };
    let roles = |sid: &str| -> Vec<String> {
        roles_of
            .get(sid)
            .into_iter()
            .flatten()
            .map(|r| (*r).to_owned())
            .collect()
    };
    let mut groups: Vec<Group> = own
        .iter()
        .map(|(id, name, description)| {
            let sid = own_sid(*id);
            Group {
                members: members
                    .iter()
                    .filter(|(group, ..)| group == id)
                    .map(|(_, sid, kind, name)| Member {
                        sid: sid.clone(),
                        name: names.get(sid).cloned().unwrap_or_else(|| name.clone()),
                        kind: kind.clone(),
                        user_id: user_ids.get(sid).copied(),
                    })
                    .collect(),
                member_of: Vec::new(),
                roles: roles(&sid),
                grants: grants.get(&sid).copied().unwrap_or(0),
                name: name.clone(),
                description: description.clone(),
                id: Some(*id),
                source: "own",
                sid,
            }
        })
        .collect();
    groups.extend(directory_members.into_iter().map(|(sid, members)| Group {
        name: label(&sid),
        member_of: member_of(&sid),
        roles: roles(&sid),
        grants: grants.get(&sid).copied().unwrap_or(0),
        members,
        id: None,
        description: String::new(),
        source: "directory",
        sid,
    }));
    groups.sort_by_key(|g| g.name.to_lowercase());
    Ok(Json(Overview { users, groups }))
}

#[derive(Deserialize)]
pub struct Of {
    principal: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct GrantRow {
    id: Uuid,
    folder_id: Option<Uuid>,
    device_id: Option<Uuid>,
    credential_id: Option<Uuid>,
    role: String,
    expires_at: Option<String>,
}

#[derive(Serialize)]
pub struct PrincipalGrant {
    id: Uuid,
    object: remotehub_model::ObjectId,
    name: String,
    /// The folders above it, from the top.
    path: Vec<String>,
    role: String,
    /// When it ends by itself.
    expires_at: Option<String>,
}

/// The grants of one user or group that have not ended, with where they lie.
pub async fn grants(
    State(state): State<AppState>,
    session: Session,
    Query(of): Query<Of>,
) -> Result<Json<Vec<PrincipalGrant>>, Problem> {
    require_auditor(&session)?;
    let rows: Vec<GrantRow> = sqlx::query_as(
        "SELECT id, folder_id, device_id, credential_id, role,
                to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS expires_at
         FROM grants WHERE principal_sid = $1 AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(&of.principal)
    .fetch_all(&state.db)
    .await?;
    let catalog = crate::catalog::load(&state.db).await?;
    let names = Names::load(&state).await?;
    let mut grants: Vec<PrincipalGrant> = rows
        .into_iter()
        .filter_map(|row| {
            let object =
                crate::catalog::object_id(row.folder_id, row.device_id, row.credential_id)?;
            Some(PrincipalGrant {
                id: row.id,
                object,
                name: names.name(object),
                path: names.path(catalog.parent(object)),
                role: row.role,
                expires_at: row.expires_at,
            })
        })
        .collect();
    grants.sort_by(|a, b| (&a.path, &a.name).cmp(&(&b.path, &b.name)));
    Ok(Json(grants))
}
