//! Permission reports for auditors and administrators (#110): what a user
//! can reach and why, and who can reach a folder. `authorize()` decides as
//! always; the reports name the grants behind its answer
//! (`Catalog::reasons`, `Catalog::grants_along`).
//!
//! - `GET /api/reports/people`, `GET /api/reports/folders`: whom and what a
//!   report can be about, for people who see neither otherwise.
//! - `GET /api/reports/users/{id}`: every object the user reaches, with
//!   their role and the grants that give it.
//! - `GET /api/reports/folders/{id}`: the grants on the folder and above it,
//!   with the members of remotehub's own groups.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use remotehub_directory::Sid;
use remotehub_model::{Catalog, ObjectId, Role};
use serde::Serialize;
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use crate::session::Session;
use crate::{AppState, catalog};

fn require_auditor(session: &Session) -> Result<(), Problem> {
    if session.is_auditor() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Person {
    id: Uuid,
    username: String,
    display_name: String,
    kind: String,
}

pub async fn people(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<Person>>, Problem> {
    require_auditor(&session)?;
    let people = sqlx::query_as(
        "SELECT id, username, display_name, kind FROM users ORDER BY lower(display_name), id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(people))
}

/// Names of everything in the catalog, for the reports' paths.
pub(super) struct Names {
    folders: HashMap<Uuid, (Option<Uuid>, String)>,
    objects: HashMap<ObjectId, String>,
}

impl Names {
    pub(super) async fn load(state: &AppState) -> Result<Self, Problem> {
        let folders: Vec<(Uuid, Option<Uuid>, String)> =
            sqlx::query_as("SELECT id, parent_id, name FROM folders")
                .fetch_all(&state.db)
                .await?;
        let devices: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM devices")
            .fetch_all(&state.db)
            .await?;
        let credentials: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM credentials")
            .fetch_all(&state.db)
            .await?;
        let objects = folders
            .iter()
            .map(|(id, _, name)| (ObjectId::Folder(*id), name.clone()))
            .chain(devices.into_iter().map(|(id, n)| (ObjectId::Device(id), n)))
            .chain(
                credentials
                    .into_iter()
                    .map(|(id, n)| (ObjectId::Credential(id), n)),
            )
            .collect();
        Ok(Names {
            folders: folders
                .into_iter()
                .map(|(id, parent, name)| (id, (parent, name)))
                .collect(),
            objects,
        })
    }

    pub(super) fn name(&self, object: ObjectId) -> String {
        self.objects.get(&object).cloned().unwrap_or_default()
    }

    /// The names of the folders from the top down to `folder`.
    pub(super) fn path(&self, folder: Option<Uuid>) -> Vec<String> {
        let mut names = Vec::new();
        let mut next = folder;
        // A bound instead of loop detection: the database prevents loops.
        while let Some(id) = next
            && names.len() < 100
        {
            let Some((parent, name)) = self.folders.get(&id) else {
                break;
            };
            names.push(name.clone());
            next = *parent;
        }
        names.reverse();
        names
    }
}

#[derive(Serialize)]
pub struct FolderChoice {
    id: Uuid,
    path: Vec<String>,
}

pub async fn folders(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Vec<FolderChoice>>, Problem> {
    require_auditor(&session)?;
    let names = Names::load(&state).await?;
    let mut folders: Vec<FolderChoice> = names
        .folders
        .keys()
        .map(|id| FolderChoice {
            id: *id,
            path: names.path(Some(*id)),
        })
        .collect();
    folders.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Json(folders))
}

/// Display names of the principals grants and memberships name: today's
/// name of a user or group remotehub knows, else the one a grant or
/// membership kept from when it was made.
pub(super) async fn principal_names(state: &AppState) -> Result<HashMap<String, String>, Problem> {
    // The names that count most come last and so win in the map.
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT sid, name FROM (
             SELECT principal_sid AS sid, principal_name AS name, 2 AS rank FROM grants
             UNION ALL SELECT principal_sid, principal_name, 2 FROM group_members
             UNION ALL SELECT sid, display_name, 1 FROM users WHERE sid IS NOT NULL
             UNION ALL SELECT 'local:' || identity_id, display_name, 1 FROM users
                 WHERE identity_id IS NOT NULL
             UNION ALL SELECT 'group:' || id, name, 1 FROM groups
             UNION ALL SELECT sid, name, 1 FROM directory_groups
         ) named ORDER BY rank DESC",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().collect())
}

#[derive(Serialize)]
pub struct Reason {
    principal_sid: String,
    principal_name: String,
    role: Role,
    /// The object the grant lies on.
    on: ObjectId,
    on_name: String,
    /// On a folder above the object, not on the object itself.
    inherited: bool,
}

#[derive(Serialize)]
pub struct Reach {
    object: ObjectId,
    name: String,
    /// The folders above it, from the top.
    path: Vec<String>,
    role: Role,
    /// Empty for an administrator, who reaches everything.
    reasons: Vec<Reason>,
}

#[derive(Serialize)]
pub struct UserReport {
    user: Person,
    administrator: bool,
    /// Where the directory groups come from: `directory` (asked now),
    /// `last_sign_in` (the directory was not reachable), or `none`.
    groups_from: &'static str,
    /// Every SID the report counts as the user's: own, directory groups,
    /// groups of remotehub's own.
    principals: Vec<(String, String)>,
    reach: Vec<Reach>,
}

/// The directory groups of a user: asked from the directory now, or those
/// of their last sign-in.
async fn directory_groups(
    state: &AppState,
    user: Uuid,
    sid: Option<&str>,
) -> Result<(Vec<String>, &'static str), Problem> {
    if let (Some(directory), Some(sid)) = (
        state.directory.get(),
        sid.and_then(|s| s.parse::<Sid>().ok()),
    ) {
        match directory.refresh(&sid).await {
            Ok(groups) => {
                return Ok((
                    groups.iter().map(ToString::to_string).collect(),
                    "directory",
                ));
            }
            Err(error) => {
                tracing::warn!(%error, %user, "the report takes the groups of the last sign-in")
            }
        }
    }
    let last: Option<Vec<String>> = sqlx::query_scalar(
        "SELECT groups FROM sessions WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user)
    .fetch_optional(&state.db)
    .await?;
    Ok(match last {
        Some(groups) => (groups, "last_sign_in"),
        None => (Vec::new(), "none"),
    })
}

pub async fn user(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<UserReport>, Problem> {
    require_auditor(&session)?;
    type Row = (Uuid, String, String, String, Option<String>, Option<Uuid>);
    let (user_id, username, display_name, kind, sid, identity): Row = sqlx::query_as(
        "SELECT id, username, display_name, kind, sid, identity_id FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))?;
    let (groups, groups_from) = if kind == "directory" {
        directory_groups(&state, user_id, sid.as_deref()).await?
    } else {
        (Vec::new(), "none")
    };
    // The user as their next session would be read: memberships and roles
    // as in `session::lookup`, so `is_admin` and `subject` decide as there.
    let mut them = Session {
        token_hash: Vec::new(),
        user_id,
        username: username.clone(),
        display_name: display_name.clone(),
        kind: kind.clone(),
        sid: sid.clone(),
        upn: None,
        groups,
        identity_id: identity,
        memberships: Vec::new(),
        roles: Vec::new(),
    };
    let own: Vec<String> = them.principal().into_iter().collect();
    let named: Vec<String> = them.groups.iter().cloned().chain(own).collect();
    them.memberships = sqlx::query_scalar(
        "SELECT DISTINCT 'group:' || group_id FROM group_members WHERE principal_sid = ANY ($1)",
    )
    .bind(&named)
    .fetch_all(&state.db)
    .await?;
    them.roles = sqlx::query_scalar(
        "SELECT DISTINCT role FROM role_assignments WHERE principal_sid = ANY ($1)",
    )
    .bind(them.sids())
    .fetch_all(&state.db)
    .await?;
    let subject = them.subject();
    let administrator = subject.admin;

    let catalog: Catalog = catalog::load(&state.db).await?;
    let names = Names::load(&state).await?;
    let principals = principal_names(&state).await?;
    let label = |sid: &str| {
        principals
            .get(sid)
            .cloned()
            .unwrap_or_else(|| sid.to_owned())
    };
    let mut reach: Vec<Reach> = catalog
        .visible(&subject)
        .roles
        .into_iter()
        .map(|(object, role)| {
            let reasons = if administrator {
                Vec::new()
            } else {
                catalog
                    .reasons(&subject, object)
                    .into_iter()
                    .map(|(on, principal, role)| Reason {
                        principal_sid: principal.to_owned(),
                        principal_name: label(principal),
                        role,
                        on,
                        on_name: names.name(on),
                        inherited: on != object,
                    })
                    .collect()
            };
            Reach {
                object,
                name: names.name(object),
                path: names.path(catalog.parent(object)),
                role,
                reasons,
            }
        })
        .collect();
    reach.sort_by(|a, b| (&a.path, &a.name).cmp(&(&b.path, &b.name)));
    let principals = subject
        .sids
        .iter()
        .map(|sid| (sid.clone(), label(sid)))
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_iter()
        .collect();
    Ok(Json(UserReport {
        user: Person {
            id: user_id,
            username,
            display_name,
            kind,
        },
        administrator,
        groups_from,
        principals,
        reach,
    }))
}

#[derive(Serialize)]
pub struct Holder {
    principal_sid: String,
    principal_name: String,
    role: Role,
    on: Uuid,
    on_path: Vec<String>,
    inherited: bool,
    /// For a group of remotehub's own: its members, by name.
    members: Vec<String>,
}

#[derive(Serialize)]
pub struct FolderReport {
    folder: FolderChoice,
    holders: Vec<Holder>,
}

pub async fn folder(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Json<FolderReport>, Problem> {
    require_auditor(&session)?;
    let catalog: Catalog = catalog::load(&state.db).await?;
    let object = ObjectId::Folder(id);
    if !catalog.contains(object) {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let names = Names::load(&state).await?;
    let principals = principal_names(&state).await?;
    let members: Vec<(String, String)> = sqlx::query_as(
        "SELECT 'group:' || group_id, principal_name FROM group_members ORDER BY principal_name",
    )
    .fetch_all(&state.db)
    .await?;
    let holders = catalog
        .grants_along(object)
        .into_iter()
        .filter_map(|(on, principal, role)| {
            let ObjectId::Folder(on) = on else {
                return None;
            };
            Some(Holder {
                principal_sid: principal.to_owned(),
                principal_name: principals
                    .get(principal)
                    .cloned()
                    .unwrap_or_else(|| principal.to_owned()),
                role,
                on,
                on_path: names.path(Some(on)),
                inherited: on != id,
                members: members
                    .iter()
                    .filter(|(group, _)| group == principal)
                    .map(|(_, name)| name.clone())
                    .collect(),
            })
        })
        .collect();
    Ok(Json(FolderReport {
        folder: FolderChoice {
            id,
            path: names.path(Some(id)),
        },
        holders,
    }))
}
