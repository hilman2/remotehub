//! Folders, devices, credentials and grants (`/api/tree`, `/api/folders`,
//! `/api/devices`, `/api/credentials`, `/api/grants`, `/api/directory`).
//! Login profiles are in `profiles.rs`.
//!
//! Every handler asks `authorize()` (crates/model) first and records what it
//! changed in the audit log, in the same transaction. Objects a user cannot
//! see answer `not_found`, so their existence does not leak.

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use remotehub_gateway::guacamole::KEYBOARD_LAYOUTS;
use remotehub_gateway::ssh::{KeyError, SshKey};
use remotehub_model::{Catalog, ObjectId, Role, Subject};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::fields::{self, Field, FieldInput};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::audit::{self, Action, Actor, Entry};
use crate::principal::{self, Principal, PrincipalId};
use crate::session::Session;
use crate::{AppState, catalog, secrets};

pub(super) const PASSWORD_FIELD: &str = "password";

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(super) async fn context(
    state: &AppState,
    session: &Session,
) -> Result<(Subject, Catalog), Problem> {
    Ok((session.subject(), catalog::load(&state.db).await?))
}

/// `not_found` for objects the subject cannot see at all, `forbidden` for
/// objects they see but may not act on in this way.
pub(super) fn require(
    catalog: &Catalog,
    subject: &Subject,
    needed: Role,
    object: ObjectId,
) -> Result<(), Problem> {
    match catalog.effective_role(subject, object) {
        Some(role) if role >= needed => Ok(()),
        Some(_) => Err(Problem::new(ErrorCode::Forbidden)),
        None => Err(Problem::new(ErrorCode::NotFound)),
    }
}

pub(super) fn body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Problem> {
    body.map(|Json(value)| value)
        .map_err(|_| Problem::new(ErrorCode::InvalidRequest))
}

pub(super) fn invalid(field: &str) -> Problem {
    Problem::new(ErrorCode::InvalidRequest).param("field", field)
}

/// A trimmed, non-empty name of at most 200 characters.
pub(super) fn name(value: &str, field: &str) -> Result<String, Problem> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 200 || value.chars().any(char::is_control) {
        return Err(invalid(field));
    }
    Ok(value.to_owned())
}

/// Unique names and non-empty folders become their own problems.
pub(super) fn database(error: sqlx::Error) -> Problem {
    if let Some(db) = error.as_database_error() {
        if db.is_unique_violation() {
            return Problem::new(ErrorCode::NameTaken);
        }
        // ON DELETE RESTRICT reports restrict_violation (23001), not a
        // foreign key violation.
        if db.is_foreign_key_violation() || db.code().as_deref() == Some("23001") {
            return Problem::new(ErrorCode::FolderNotEmpty);
        }
    }
    Problem::from(error)
}

pub(super) fn entry<'a>(
    session: &'a Session,
    action: Action,
    object: ObjectId,
    details: serde_json::Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: Some((catalog::kind(object), catalog::id(object))),
        details,
        address: Some(address),
    }
}

/// `Option<Option<T>>` for JSON: absent → `None`, `null` → `Some(None)`.
pub(super) fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

// ── Tree ────────────────────────────────────────────────────────────────────

/// The login profiles, as `ProfileRow` reads them.
const PROFILES: &str = "SELECT id, folder_id, name, username, domain, secret_kind,
        key_algorithm, key_fingerprint, has_certificate,
        to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS updated_at
     FROM login_profiles ORDER BY lower(name)";

#[derive(Serialize)]
pub struct Tree {
    folders: Vec<TreeFolder>,
    devices: Vec<TreeDevice>,
    /// Logins that devices share (#192).
    profiles: Vec<TreeProfile>,
    /// Where shared credentials live, apart from the folders (#190).
    collections: Vec<TreeCollection>,
    credentials: Vec<TreeCredential>,
    may_create_top_level: bool,
    /// The visible folders this user has open in the tree; the rest are
    /// closed.
    open: Vec<Uuid>,
    /// Whether this user states a purpose before every connection (#90).
    purpose_required: bool,
}

#[derive(Serialize)]
struct TreeFolder {
    id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    /// `None`: only shown as the way to something visible inside.
    role: Option<Role>,
    /// The site connector the folder names for the devices below it (#176);
    /// none: its parent's.
    connector_id: Option<Uuid>,
}

#[derive(Serialize)]
struct TreeCollection {
    id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    /// `None`: only shown as the way to something visible inside.
    role: Option<Role>,
}

#[derive(Serialize, sqlx::FromRow)]
struct DeviceRow {
    id: Uuid,
    folder_id: Uuid,
    name: String,
    protocol: String,
    host: String,
    port: i32,
    auth_mode: String,
    /// Sign-in mode `profile`: the login profile it uses (#192).
    profile_id: Option<Uuid>,
    description: String,
    /// Words everyone who sees the device may change, for everyone's search
    /// (#215); who changed them last, by name, and when.
    keywords: String,
    keywords_changed_by: Option<String>,
    keywords_changed_at: Option<String>,
    /// RDP only; `None` uses the instance's default.
    keyboard_layout: Option<String>,
    /// RDP and HTTPS: SHA-256 fingerprint of the pinned certificate, if one
    /// is pinned.
    certificate_fingerprint: Option<String>,
    /// `inherit` from the folder, `direct`, or its own `connector` (#176).
    connector_mode: String,
    /// Its own connector, with `connector_mode` `connector`.
    connector_id: Option<Uuid>,
    /// The connector it is reached through, however chosen; none: directly.
    reached_through: Option<Uuid>,
    /// Sign-in mode `device`: its own credentials as far as they may be
    /// shown: user name, domain, `password` or `ssh_key`, and what
    /// identifies a key. Password and key stay sealed.
    username: String,
    domain: String,
    secret_kind: String,
    key_algorithm: Option<String>,
    key_fingerprint: Option<String>,
    has_certificate: bool,
    #[serde(skip)]
    host_key: Option<String>,
}

#[derive(Serialize)]
struct TreeDevice {
    #[serde(flatten)]
    device: DeviceRow,
    /// SHA-256 fingerprint of the pinned host key, if one is pinned.
    host_key_fingerprint: Option<String>,
    role: Role,
}

/// A login profile as the tree shows it: what identifies it, never its
/// password or key.
#[derive(Serialize, sqlx::FromRow)]
struct ProfileRow {
    id: Uuid,
    /// None: at the top level, for administrators only.
    folder_id: Option<Uuid>,
    name: String,
    username: String,
    domain: String,
    /// `password` or `ssh_key`.
    secret_kind: String,
    key_algorithm: Option<String>,
    key_fingerprint: Option<String>,
    has_certificate: bool,
    updated_at: String,
}

#[derive(Serialize)]
struct TreeProfile {
    #[serde(flatten)]
    profile: ProfileRow,
    role: Role,
}

#[derive(Serialize, sqlx::FromRow)]
struct CredentialRow {
    id: Uuid,
    collection_id: Uuid,
    name: String,
    username: String,
    version: i32,
    url: String,
    notes: String,
    /// One of KeePass' standard icons.
    icon: i16,
    /// Custom fields; protected ones without their value.
    fields: sqlx::types::Json<Vec<Field>>,
    /// Files kept with it (#100), without their content.
    attachments: sqlx::types::Json<Vec<Attachment>>,
    /// Words to find it by (#193).
    tags: Vec<String>,
    /// `YYYY-MM-DD`: the day its password runs out; none: never.
    expires_on: Option<String>,
    /// Whether a one-time password is sealed with it; its codes come from
    /// `POST /api/credentials/{id}/code`.
    has_totp: bool,
    /// RFC 3339, UTC.
    updated_at: String,
    /// RFC 3339, UTC: in the recycle bin since; none: in its collection.
    deleted_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Attachment {
    id: Uuid,
    name: String,
    size: i64,
}

#[derive(Serialize)]
struct TreeCredential {
    #[serde(flatten)]
    credential: CredentialRow,
    role: Role,
}

/// Everything the user may see, with their role on each object.
pub async fn tree(State(state): State<AppState>, session: Session) -> Result<Json<Tree>, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    let visible = catalog.visible(&subject);

    let folders: Vec<(Uuid, Option<Uuid>, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, parent_id, name, connector_id FROM folders ORDER BY lower(name)",
    )
    .fetch_all(&state.db)
    .await?;
    let devices: Vec<DeviceRow> = sqlx::query_as(
        "SELECT id, folder_id, name, protocol, host, port, auth_mode, profile_id, description,
                keywords, keywords_changed_by,
                to_char(keywords_changed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                    AS keywords_changed_at,
                keyboard_layout, certificate_fingerprint, connector_mode, connector_id,
                device_connector(connector_mode, connector_id, folder_id) AS reached_through,
                username, domain, secret_kind, key_algorithm, key_fingerprint, has_certificate,
                host_key
         FROM devices ORDER BY lower(name)",
    )
    .fetch_all(&state.db)
    .await?;
    let profiles: Vec<ProfileRow> = sqlx::query_as(PROFILES).fetch_all(&state.db).await?;
    let credentials: Vec<CredentialRow> = sqlx::query_as(
        "SELECT c.id, c.collection_id, c.name, c.username, c.version,
                c.url, c.notes, c.icon, c.fields,
                coalesce((SELECT json_agg(json_build_object('id', a.id, 'name', a.name,
                                                            'size', a.size) ORDER BY a.name)
                          FROM credential_attachments a WHERE a.credential_id = c.id),
                         '[]') AS attachments,
                c.tags, c.expires_on::text AS expires_on, c.has_totp,
                to_char(c.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                    AS updated_at,
                to_char(c.deleted_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')
                    AS deleted_at
         FROM credentials c ORDER BY lower(c.name)",
    )
    .fetch_all(&state.db)
    .await?;
    let collections: Vec<(Uuid, Option<Uuid>, String)> =
        sqlx::query_as("SELECT id, parent_id, name FROM collections ORDER BY lower(name)")
            .fetch_all(&state.db)
            .await?;
    let open: Vec<Uuid> =
        sqlx::query_scalar("SELECT folder_id FROM open_folders WHERE user_id = $1")
            .bind(session.user_id)
            .fetch_all(&state.db)
            .await?;
    let sees = |id: &Uuid| {
        visible.roles.contains_key(&ObjectId::Folder(*id)) || visible.path_only.contains(id)
    };
    let open = open.into_iter().filter(sees).collect();
    let purpose_required = super::connect::purpose_required(&state.db, &session).await?;

    Ok(Json(Tree {
        open,
        purpose_required,
        folders: folders
            .into_iter()
            .filter_map(|(id, parent_id, name, connector_id)| {
                let role = visible.roles.get(&ObjectId::Folder(id)).copied();
                (role.is_some() || visible.path_only.contains(&id)).then_some(TreeFolder {
                    id,
                    parent_id,
                    name,
                    role,
                    connector_id,
                })
            })
            .collect(),
        devices: devices
            .into_iter()
            .filter_map(|device| {
                let role = *visible.roles.get(&ObjectId::Device(device.id))?;
                let host_key_fingerprint = device
                    .host_key
                    .as_deref()
                    .and_then(remotehub_gateway::ssh::fingerprint);
                Some(TreeDevice {
                    device,
                    host_key_fingerprint,
                    role,
                })
            })
            .collect(),
        profiles: profiles
            .into_iter()
            .filter_map(|profile| {
                let role = *visible.roles.get(&ObjectId::Profile(profile.id))?;
                Some(TreeProfile { profile, role })
            })
            .collect(),
        collections: collections
            .into_iter()
            .filter_map(|(id, parent_id, name)| {
                let role = visible.roles.get(&ObjectId::Collection(id)).copied();
                (role.is_some() || visible.collections_path_only.contains(&id)).then_some(
                    TreeCollection {
                        id,
                        parent_id,
                        name,
                        role,
                    },
                )
            })
            .collect(),
        credentials: credentials
            .into_iter()
            .filter_map(|credential| {
                let role = *visible.roles.get(&ObjectId::Credential(credential.id))?;
                Some(TreeCredential { credential, role })
            })
            .collect(),
        may_create_top_level: catalog.may_create_top_level(&subject),
    }))
}

// ── Folders ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct FolderOpen {
    open: bool,
}

/// `PUT /api/folders/{id}/open`: opens or closes a folder in the caller's
/// tree (#83). A preference of this user only, not audited.
pub async fn set_folder_open(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
    input: Result<Json<FolderOpen>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let (subject, catalog) = context(&state, &session).await?;
    let visible = catalog.visible(&subject);
    if !visible.roles.contains_key(&ObjectId::Folder(id)) && !visible.path_only.contains(&id) {
        return Err(Problem::new(ErrorCode::NotFound));
    }
    let query = if input.open {
        "INSERT INTO open_folders (user_id, folder_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
    } else {
        "DELETE FROM open_folders WHERE user_id = $1 AND folder_id = $2"
    };
    sqlx::query(query)
        .bind(session.user_id)
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct NewFolder {
    parent_id: Option<Uuid>,
    name: String,
    /// The site connector for what is in it (#176); none: its parent's.
    #[serde(default)]
    connector_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct Created {
    id: Uuid,
}

impl Created {
    pub(super) fn new(id: Uuid) -> Self {
        Created { id }
    }
}

pub async fn create_folder(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewFolder>, JsonRejection>,
) -> Result<(StatusCode, Json<Created>), Problem> {
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let (subject, catalog) = context(&state, &session).await?;
    match input.parent_id {
        None if catalog.may_create_top_level(&subject) => {}
        None => return Err(Problem::new(ErrorCode::Forbidden)),
        Some(parent) => require(&catalog, &subject, Role::Manage, ObjectId::Folder(parent))?,
    }
    // A new folder holds no devices yet: no target changes.
    require_connector(&state, input.connector_id).await?;

    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO folders (parent_id, name, connector_id) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(input.parent_id)
    .bind(&name)
    .bind(input.connector_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    let details = json!({
        "name": name, "parent_id": input.parent_id, "connector_id": input.connector_id,
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::FolderCreated,
            ObjectId::Folder(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

#[derive(Deserialize)]
pub struct FolderChange {
    name: Option<String>,
    /// Absent: stays; `null`: to the top level; an id: into that folder.
    #[serde(default, deserialize_with = "nullable")]
    parent_id: Option<Option<Uuid>>,
    /// Absent: stays; `null`: the parent's; an id: that connector (#176).
    #[serde(default, deserialize_with = "nullable")]
    connector_id: Option<Option<Uuid>>,
}

/// A device in a folder's subtree, and the connector it is reached through.
#[derive(sqlx::FromRow)]
struct Reached {
    id: Uuid,
    name: String,
    profile_id: Option<Uuid>,
    reached_through: Option<Uuid>,
}

/// The devices whose login profile no longer lies in their folder or one
/// above it, by name. A device may use only a profile within its reach
/// (#192); a move of a folder or a profile must not take it out of reach.
pub(super) async fn out_of_reach(tx: &mut sqlx::PgConnection) -> Result<Vec<String>, Problem> {
    Ok(sqlx::query_scalar(
        "SELECT d.name FROM devices d JOIN login_profiles p ON p.id = d.profile_id
         WHERE p.folder_id IS NOT NULL AND NOT folder_within(d.folder_id, p.folder_id)
         ORDER BY lower(d.name)",
    )
    .fetch_all(tx)
    .await?)
}

/// Every device in `folder` and the folders below it, with its connector.
async fn devices_below(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    folder: Uuid,
) -> Result<Vec<Reached>, Problem> {
    Ok(sqlx::query_as(
        "WITH RECURSIVE below AS (
             SELECT id FROM folders WHERE id = $1
             UNION ALL
             SELECT f.id FROM folders f JOIN below b ON f.parent_id = b.id
         )
         SELECT d.id, d.name, d.profile_id,
                device_connector(d.connector_mode, d.connector_id, d.folder_id) AS reached_through
         FROM devices d WHERE d.folder_id IN (SELECT id FROM below)",
    )
    .bind(folder)
    .fetch_all(&mut **tx)
    .await?)
}

/// Forgets what was pinned for devices whose target changed: the same
/// address behind another connector may be another machine.
const FORGET_PINS: &str = "UPDATE devices SET host_key = NULL, host_key_pinned_at = NULL,
         certificate_fingerprint = NULL, certificate_pinned_at = NULL
     WHERE id = ANY($1)";

pub async fn update_folder(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<FolderChange>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, ObjectId::Folder(id))?;
    let name = input.name.as_deref().map(|n| name(n, "name")).transpose()?;
    if let Some(target) = input.parent_id {
        match target {
            None if catalog.may_create_top_level(&subject) => {}
            None => return Err(Problem::new(ErrorCode::Forbidden)),
            Some(parent) => {
                require(&catalog, &subject, Role::Manage, ObjectId::Folder(parent))?;
                if catalog.is_within(parent, id) {
                    return Err(invalid("parent_id"));
                }
            }
        }
    }
    if let Some(Some(connector)) = input.connector_id {
        require_connector(&state, Some(connector)).await?;
    }

    let mut tx = state.db.begin().await?;
    let before = devices_below(&mut tx, id).await?;
    sqlx::query(
        "UPDATE folders SET
             name = coalesce($2, name),
             parent_id = CASE WHEN $3 THEN $4 ELSE parent_id END,
             connector_id = CASE WHEN $5 THEN $6 ELSE connector_id END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(&name)
    .bind(input.parent_id.is_some())
    .bind(input.parent_id.flatten())
    .bind(input.connector_id.is_some())
    .bind(input.connector_id.flatten())
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    // A new connector here, or one above after a move, is another target for
    // every device below that inherits it: the same rules as changing one
    // device's target (see update_device). Their own passwords and keys go
    // along: managing the folder includes `reveal` on everything in it.
    let after = devices_below(&mut tx, id).await?;
    let retargeted: Vec<&Reached> = after
        .iter()
        .filter(|now| {
            before
                .iter()
                .any(|was| was.id == now.id && was.reached_through != now.reached_through)
        })
        .collect();
    let refused: Vec<&str> = retargeted
        .iter()
        .filter(|device| {
            device.profile_id.is_some_and(|profile| {
                require(
                    &catalog,
                    &subject,
                    Role::Connect,
                    ObjectId::Profile(profile),
                )
                .is_err()
            })
        })
        .map(|device| device.name.as_str())
        .collect();
    if !refused.is_empty() {
        return Err(Problem::new(ErrorCode::RetargetForbidden).param("devices", refused.join(", ")));
    }
    let stranded = out_of_reach(&mut tx).await?;
    if !stranded.is_empty() {
        return Err(
            Problem::new(ErrorCode::ProfileOutOfReach).param("devices", stranded.join(", "))
        );
    }
    let retargeted: Vec<Uuid> = retargeted.iter().map(|device| device.id).collect();
    sqlx::query(FORGET_PINS)
        .bind(&retargeted)
        .execute(&mut *tx)
        .await?;
    let details = json!({
        "name": name, "parent_id": input.parent_id, "connector_id": input.connector_id,
        "retargeted": retargeted,
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::FolderUpdated,
            ObjectId::Folder(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_folder(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, ObjectId::Folder(id))?;
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM folders WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::FolderDeleted,
            ObjectId::Folder(id),
            json!({}),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Devices ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DeviceInput {
    folder_id: Uuid,
    name: String,
    protocol: String,
    host: String,
    port: u16,
    auth_mode: String,
    /// Sign-in mode `profile` only: the login profile it uses.
    #[serde(default)]
    profile_id: Option<Uuid>,
    #[serde(default)]
    description: String,
    /// RDP only: one of guacd's layouts, or none for the instance's default.
    #[serde(default)]
    keyboard_layout: Option<String>,
    /// How the device is reached (#176): `inherit` from its folder (the
    /// default), `direct`, or through its own `connector`, `connector_id`.
    #[serde(default)]
    connector_mode: Option<String>,
    #[serde(default)]
    connector_id: Option<Uuid>,
    /// Sign-in mode `device` only: the device's own credentials, a password
    /// (default) or, for SSH, a key (`ssh_key`).
    #[serde(default)]
    username: String,
    #[serde(default)]
    domain: String,
    #[serde(default)]
    secret_kind: Option<String>,
    #[serde(flatten)]
    secrets: SecretInput,
}

struct ValidDevice {
    folder_id: Uuid,
    name: String,
    protocol: &'static str,
    host: String,
    port: i32,
    auth_mode: &'static str,
    profile_id: Option<Uuid>,
    description: String,
    keyboard_layout: Option<&'static str>,
    connector_mode: &'static str,
    /// Set exactly with `connector_mode` `connector`.
    connector_id: Option<Uuid>,
    /// Empty unless the sign-in mode is `device`.
    username: String,
    domain: String,
    /// `password` unless the device's own credentials are a key.
    secret_kind: &'static str,
    /// New secrets of the device's own credentials, if given.
    secrets: Option<Secrets>,
}

impl DeviceInput {
    fn validate(self) -> Result<ValidDevice, Problem> {
        let connector_mode = match (self.connector_mode.as_deref(), self.connector_id) {
            (None | Some("inherit"), None) => "inherit",
            (Some("direct"), None) => "direct",
            (Some("connector"), Some(_)) => "connector",
            _ => return Err(invalid("connector_id")),
        };
        let protocol = match self.protocol.as_str() {
            "ssh" => "ssh",
            "rdp" => "rdp",
            "vnc" => "vnc",
            "https" => "https",
            _ => return Err(invalid("protocol")),
        };
        let auth_mode = match self.auth_mode.as_str() {
            "profile" => "profile",
            "ask" => "ask",
            "own" => "own",
            // Certificates are SSH's own; RDP and VNC know nothing like it.
            "certificate" if protocol == "ssh" => "certificate",
            // LAPS keeps the password of a computer's local administrator:
            // not VNC's own password, nor a web interface's.
            "laps" if protocol == "ssh" || protocol == "rdp" => "laps",
            "device" => "device",
            _ => return Err(invalid("auth_mode")),
        };
        let own = auth_mode == "device";
        let secret_kind = match self.secret_kind.as_deref().filter(|_| own) {
            None | Some("password") => "password",
            // Keys are SSH's; RDP, VNC and web interfaces take passwords.
            Some("ssh_key") if protocol == "ssh" => "ssh_key",
            _ => return Err(invalid("secret_kind")),
        };
        let given = self.secrets.given();
        let secrets = if own {
            new_secrets(secret_kind, &given)?
        } else {
            None
        };
        // A login profile is exactly what "profile" means, and nothing else.
        if (auth_mode == "profile") != self.profile_id.is_some() {
            return Err(invalid("profile_id"));
        }
        let host = self.host.trim();
        if host.is_empty()
            || host.len() > 253
            || host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "/\\@?#".contains(c))
        {
            return Err(invalid("host"));
        }
        if self.port == 0 {
            return Err(invalid("port"));
        }
        if self.description.chars().count() > 2000 {
            return Err(invalid("description"));
        }
        // Other protocols send characters, not scancodes: no layout to keep.
        let keyboard_layout = match self.keyboard_layout.as_deref().filter(|l| !l.is_empty()) {
            Some(layout) if protocol == "rdp" => Some(
                *KEYBOARD_LAYOUTS
                    .iter()
                    .find(|known| **known == layout)
                    .ok_or_else(|| invalid("keyboard_layout"))?,
            ),
            _ => None,
        };
        Ok(ValidDevice {
            folder_id: self.folder_id,
            name: name(&self.name, "name")?,
            protocol,
            host: host.to_owned(),
            port: i32::from(self.port),
            auth_mode,
            profile_id: self.profile_id,
            description: self.description.trim().to_owned(),
            keyboard_layout,
            connector_mode,
            connector_id: self.connector_id,
            username: if own {
                plain(&self.username, "username")?
            } else {
                String::new()
            },
            domain: if own {
                plain(&self.domain, "domain")?
            } else {
                String::new()
            },
            secret_kind,
            secrets,
        })
    }
}

/// A device as it is stored, before a change.
#[derive(sqlx::FromRow)]
struct DeviceBefore {
    protocol: String,
    host: String,
    port: i32,
    profile_id: Option<Uuid>,
    /// The connector it was reached through, however chosen.
    reached_through: Option<Uuid>,
    auth_mode: String,
    secret_version: i32,
    secret_kind: String,
    key_algorithm: Option<String>,
    key_fingerprint: Option<String>,
    has_certificate: bool,
}

/// A device in `folder` may use only a login profile in that folder or one
/// above it, or one at the top level (#192): a customer's profile stays
/// with the customer's devices.
fn in_reach(catalog: &Catalog, profile: Uuid, folder: Uuid) -> Result<(), Problem> {
    match catalog.parent(ObjectId::Profile(profile)) {
        Some(home) if !catalog.is_within(folder, home) => {
            Err(Problem::new(ErrorCode::ProfileOutOfReach))
        }
        _ => Ok(()),
    }
}

/// Keys are SSH's: a device of another protocol cannot sign in with a login
/// profile that holds a key.
async fn profile_fits(state: &AppState, profile: Uuid, protocol: &str) -> Result<(), Problem> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT secret_kind FROM login_profiles WHERE id = $1")
            .bind(profile)
            .fetch_optional(&state.db)
            .await?;
    if kind.as_deref() == Some("ssh_key") && protocol != "ssh" {
        return Err(invalid("profile_id"));
    }
    Ok(())
}

/// The field a device's own credentials or a login profile miss when they
/// have no secret yet.
pub(super) fn missing_secret(secret_kind: &str) -> Problem {
    invalid(if secret_kind == "ssh_key" {
        "private_key"
    } else {
        "password"
    })
}

/// The connector a device with these settings is reached through (#176).
async fn reached_through(state: &AppState, device: &ValidDevice) -> Result<Option<Uuid>, Problem> {
    Ok(sqlx::query_scalar("SELECT device_connector($1, $2, $3)")
        .bind(device.connector_mode)
        .bind(device.connector_id)
        .bind(device.folder_id)
        .fetch_one(&state.db)
        .await?)
}

/// A connector the device or folder names must exist; the foreign key would
/// only say that something is missing.
async fn require_connector(state: &AppState, connector: Option<Uuid>) -> Result<(), Problem> {
    let Some(connector) = connector else {
        return Ok(());
    };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM connectors WHERE id = $1)")
        .bind(connector)
        .fetch_one(&state.db)
        .await?;
    if exists {
        Ok(())
    } else {
        Err(invalid("connector_id"))
    }
}

pub async fn create_device(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<DeviceInput>, JsonRejection>,
) -> Result<(StatusCode, Json<Created>), Problem> {
    let device = body(input)?.validate()?;
    let (subject, catalog) = context(&state, &session).await?;
    require(
        &catalog,
        &subject,
        Role::Edit,
        ObjectId::Folder(device.folder_id),
    )?;
    // Whoever points a login profile at a host may use that profile:
    // otherwise any editor could send its password to a host of their
    // choosing.
    if let Some(profile) = device.profile_id {
        require(
            &catalog,
            &subject,
            Role::Connect,
            ObjectId::Profile(profile),
        )?;
        in_reach(&catalog, profile, device.folder_id)?;
        profile_fits(&state, profile, device.protocol).await?;
    }
    require_connector(&state, device.connector_id).await?;
    // Own credentials need their password or key from the start.
    if device.auth_mode == "device" && device.secrets.is_none() {
        return Err(missing_secret(device.secret_kind));
    }
    let secret_version = i32::from(device.secrets.is_some());
    let (key_algorithm, key_fingerprint, has_certificate) = device
        .secrets
        .as_ref()
        .map_or((None, None, false), Secrets::key_info);

    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO devices
             (folder_id, name, protocol, host, port, auth_mode, profile_id, description, keyboard_layout,
              connector_id, username, domain, secret_version, secret_kind, key_algorithm,
              key_fingerprint, has_certificate, connector_mode)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
         RETURNING id",
    )
    .bind(device.folder_id)
    .bind(&device.name)
    .bind(device.protocol)
    .bind(&device.host)
    .bind(device.port)
    .bind(device.auth_mode)
    .bind(device.profile_id)
    .bind(&device.description)
    .bind(device.keyboard_layout)
    .bind(device.connector_id)
    .bind(&device.username)
    .bind(&device.domain)
    .bind(secret_version)
    .bind(device.secret_kind)
    .bind(key_algorithm)
    .bind(key_fingerprint)
    .bind(has_certificate)
    .bind(device.connector_mode)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    if let Some(secrets) = &device.secrets {
        seal_version(&mut tx, &state, id, secret_version, secrets).await?;
    }
    let details = json!({
        "name": device.name, "protocol": device.protocol, "host": device.host, "port": device.port,
        "auth_mode": device.auth_mode, "profile_id": device.profile_id,
        "keyboard_layout": device.keyboard_layout, "connector_mode": device.connector_mode,
        "connector_id": device.connector_id,
        "username": device.username, "domain": device.domain, "secret_kind": device.secret_kind,
        "key_fingerprint": key_fingerprint,
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::DeviceCreated,
            ObjectId::Device(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

pub async fn update_device(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<DeviceInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let device = body(input)?.validate()?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Device(id))?;
    if catalog.parent(ObjectId::Device(id)) != Some(device.folder_id) {
        require(
            &catalog,
            &subject,
            Role::Edit,
            ObjectId::Folder(device.folder_id),
        )?;
    }
    let before: DeviceBefore = sqlx::query_as(
        "SELECT protocol, host, port, profile_id,
                device_connector(connector_mode, connector_id, folder_id) AS reached_through,
                auth_mode, secret_version, secret_kind, key_algorithm, key_fingerprint,
                has_certificate
         FROM devices WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    require_connector(&state, device.connector_id).await?;
    // A login profile may only be linked, or sent to a changed target, by
    // someone who may use it (see create_device). Another connector is
    // another target: the same address may be another machine at its site.
    // That includes a move into a folder with another connector (#176).
    let target_changed = before.protocol != device.protocol
        || before.host != device.host
        || before.port != device.port
        || before.reached_through != reached_through(&state, &device).await?;
    if let Some(profile) = device.profile_id {
        if before.profile_id != Some(profile) || target_changed {
            require(
                &catalog,
                &subject,
                Role::Connect,
                ObjectId::Profile(profile),
            )?;
        }
        in_reach(&catalog, profile, device.folder_id)?;
        profile_fits(&state, profile, device.protocol).await?;
    }
    // The device's own password or key goes to another target only with
    // someone who may read it (#174): pointing the device at a host of one's
    // own would hand one the password. `edit` includes `reveal` today, so
    // this holds for everyone who gets here; the audit entry says the secret
    // went along. Should the roles ever part, the others enter it again, as
    // a linked login profile needs its right to be used (above).
    let may_carry =
        !target_changed || require(&catalog, &subject, Role::Reveal, ObjectId::Device(id)).is_ok();
    let keeps = before.auth_mode == "device"
        && before.secret_version > 0
        && before.secret_kind == device.secret_kind
        && may_carry;
    let secret_kept = keeps && target_changed && device.secrets.is_none();
    let secret_version = match (&device.secrets, device.auth_mode) {
        (Some(_), _) => before.secret_version + 1,
        (None, "device") if keeps => before.secret_version,
        (None, "device") => return Err(missing_secret(device.secret_kind)),
        (None, _) => 0,
    };
    let (key_algorithm, key_fingerprint, has_certificate) = match &device.secrets {
        Some(secrets) => {
            let (algorithm, fingerprint, certificate) = secrets.key_info();
            (
                algorithm.map(str::to_owned),
                fingerprint.map(str::to_owned),
                certificate,
            )
        }
        None if secret_version > 0 => (
            before.key_algorithm,
            before.key_fingerprint,
            before.has_certificate,
        ),
        None => (None, None, false),
    };

    let mut tx = state.db.begin().await?;
    if let Some(secrets) = &device.secrets {
        seal_version(&mut tx, &state, id, secret_version, secrets).await?;
    }
    // Only the current secrets are kept; without own credentials, none.
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1 AND version <> $2")
        .bind(id)
        .bind(secret_version)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE devices SET folder_id = $2, name = $3, protocol = $4, host = $5, port = $6,
             auth_mode = $7, profile_id = $8, description = $9, keyboard_layout = $11,
             connector_id = $12, username = $13, domain = $14, secret_version = $15,
             secret_kind = $16, key_algorithm = $17, key_fingerprint = $18, has_certificate = $19,
             connector_mode = $20, updated_at = now(),
             host_key = CASE WHEN $10 THEN NULL ELSE host_key END,
             host_key_pinned_at = CASE WHEN $10 THEN NULL ELSE host_key_pinned_at END,
             certificate_fingerprint = CASE WHEN $10 THEN NULL ELSE certificate_fingerprint END,
             certificate_pinned_at = CASE WHEN $10 THEN NULL ELSE certificate_pinned_at END
         WHERE id = $1",
    )
    .bind(id)
    .bind(device.folder_id)
    .bind(&device.name)
    .bind(device.protocol)
    .bind(&device.host)
    .bind(device.port)
    .bind(device.auth_mode)
    .bind(device.profile_id)
    .bind(&device.description)
    .bind(target_changed)
    .bind(device.keyboard_layout)
    .bind(device.connector_id)
    .bind(&device.username)
    .bind(&device.domain)
    .bind(secret_version)
    .bind(device.secret_kind)
    .bind(&key_algorithm)
    .bind(&key_fingerprint)
    .bind(has_certificate)
    .bind(device.connector_mode)
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    let details = json!({
        "name": device.name, "protocol": device.protocol, "host": device.host, "port": device.port,
        "auth_mode": device.auth_mode, "profile_id": device.profile_id, "folder_id": device.folder_id,
        "keyboard_layout": device.keyboard_layout, "connector_mode": device.connector_mode,
        "connector_id": device.connector_id,
        "username": device.username, "domain": device.domain, "secret_kind": device.secret_kind,
        "key_fingerprint": key_fingerprint, "secret_changed": device.secrets.is_some(),
        "secret_kept": secret_kept,
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::DeviceUpdated,
            ObjectId::Device(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_device(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Device(id))?;
    let mut tx = state.db.begin().await?;
    // The password of its own credentials, if it has them.
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM devices WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::DeviceDeleted,
            ObjectId::Device(id),
            json!({}),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Forgets the pinned host key, so the next connection pins the key the
/// target presents then — after a deliberate key change on the target.
pub async fn reset_host_key(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Device(id))?;
    let mut tx = state.db.begin().await?;
    // Whatever is pinned goes: the SSH host key or the RDP certificate.
    let (protocol, host_key, certificate): (String, Option<String>, Option<String>) =
        sqlx::query_as(
            "UPDATE devices d SET host_key = NULL, host_key_pinned_at = NULL,
                 certificate_fingerprint = NULL, certificate_pinned_at = NULL
             FROM (SELECT host_key, certificate_fingerprint FROM devices WHERE id = $1) old
             WHERE d.id = $1 RETURNING d.protocol, old.host_key, old.certificate_fingerprint",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let (action, details) = if protocol == "rdp" {
        (
            Action::CertificateReset,
            json!({ "fingerprint": certificate }),
        )
    } else {
        let fingerprint = host_key
            .as_deref()
            .and_then(remotehub_gateway::ssh::fingerprint);
        (Action::HostKeyReset, json!({ "fingerprint": fingerprint }))
    };
    audit::record(
        &mut *tx,
        entry(&session, action, ObjectId::Device(id), details, &address),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Characters of search words at most; the column checks it too.
const MAX_KEYWORDS: usize = 500;

#[derive(Deserialize)]
pub struct KeywordsInput {
    keywords: String,
}

/// `PUT /api/devices/{id}/keywords`: the device's search words (#215).
/// Everyone who sees the device may change them, not only its editors: the
/// people who connect know the words they look for it by.
pub async fn set_keywords(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<KeywordsInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let keywords = input.keywords.trim();
    if keywords.chars().count() > MAX_KEYWORDS || keywords.chars().any(char::is_control) {
        return Err(invalid("keywords"));
    }
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::List, ObjectId::Device(id))?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE devices SET keywords = $2, keywords_changed_by = $3, keywords_changed_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(keywords)
    .bind(&session.display_name)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::DeviceKeywordsChanged,
            ObjectId::Device(id),
            json!({ "keywords": keywords }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Credentials ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CredentialInput {
    /// The collection it lives in (#190).
    collection_id: Uuid,
    name: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    icon: i16,
    #[serde(default)]
    fields: Vec<FieldInput>,
    /// Words to find it by (#193).
    #[serde(default)]
    tags: Vec<String>,
    /// `YYYY-MM-DD`; none: never runs out.
    #[serde(default)]
    expires_on: Option<String>,
    /// Its one-time password: an `otpauth://totp/` link or a base32
    /// secret. Absent on a change: the sealed one stays; empty: removed.
    #[serde(default)]
    totp: Option<SecretString>,
    #[serde(flatten)]
    secrets: SecretInput,
}

/// What a credential has next to its secrets (#98), checked.
struct Details {
    url: String,
    notes: String,
    icon: i16,
    fields: fields::Checked,
    tags: Vec<String>,
    expires_on: Option<String>,
}

/// The sealed field of a credential's one-time password (#193).
pub(super) const TOTP_FIELD: &str = "totp";

fn details(input: &CredentialInput) -> Result<Details, Problem> {
    let url = input.url.trim();
    if url.chars().count() > 2000 || url.chars().any(char::is_control) {
        return Err(invalid("url"));
    }
    let notes = input.notes.trim_end();
    if notes.chars().count() > 10_000 || notes.chars().any(|c| c.is_control() && c != '\n') {
        return Err(invalid("notes"));
    }
    if !(0..=68).contains(&input.icon) {
        return Err(invalid("icon"));
    }
    let mut tags: Vec<String> = Vec::new();
    for tag in &input.tags {
        let tag = tag.trim();
        if tag.is_empty() || tag.chars().count() > 50 || tag.chars().any(char::is_control) {
            return Err(invalid("tags"));
        }
        if !tags.iter().any(|known| known.eq_ignore_ascii_case(tag)) {
            tags.push(tag.to_owned());
        }
    }
    if tags.len() > 20 {
        return Err(invalid("tags"));
    }
    let expires_on = input
        .expires_on
        .as_deref()
        .map(str::trim)
        .filter(|day| !day.is_empty())
        .map(|day| calendar_day(day).ok_or_else(|| invalid("expires_on")))
        .transpose()?;
    Ok(Details {
        url: url.to_owned(),
        notes: notes.to_owned(),
        icon: input.icon,
        fields: fields::check(&input.fields)?,
        tags,
        expires_on,
    })
}

/// A day as `YYYY-MM-DD`, if `text` is one that exists.
fn calendar_day(text: &str) -> Option<String> {
    let mut parts = text.splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    // PostgreSQL's calendar has no year 0, and five digits no longer fit
    // the form the UI writes.
    if !(1..=9999).contains(&year) {
        return None;
    }
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    let date =
        time::Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day).ok()?;
    Some(format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    ))
}

/// A one-time password as given: None to keep the sealed one, `Some(None)`
/// to remove it, `Some(Some(text))` to seal `text`, which must read as one.
fn totp_input(input: &CredentialInput) -> Result<Option<Option<SecretString>>, Problem> {
    match &input.totp {
        None => Ok(None),
        Some(text) if text.expose_secret().trim().is_empty() => Ok(Some(None)),
        Some(text) => {
            crate::totp::Params::parse(text.expose_secret()).ok_or_else(|| invalid("totp"))?;
            Ok(Some(Some(SecretString::from(
                text.expose_secret().trim().to_owned(),
            ))))
        }
    }
}

/// Copies sealed fields of `from` to the new version `to`, for what a
/// change keeps.
async fn carry(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    id: Uuid,
    from: i32,
    to: i32,
    names: &[String],
) -> Result<(), Problem> {
    for name in names {
        let value = secrets::load(&mut *tx, &state.vault, id, from, name)
            .await
            .map_err(secret_problem)?;
        if let Some(value) = value {
            secrets::store(&mut *tx, &state.vault, id, to, name, &value)
                .await
                .map_err(secret_problem)?;
        }
    }
    Ok(())
}

/// Seals the protected fields with a new value into `version`.
async fn seal_fields(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    id: Uuid,
    version: i32,
    sealed: &[(String, SecretString)],
) -> Result<(), Problem> {
    for (name, value) in sealed {
        let field = fields::secret_name(name);
        secrets::store(
            &mut *tx,
            &state.vault,
            id,
            version,
            &field,
            value.expose_secret().as_bytes(),
        )
        .await
        .map_err(secret_problem)?;
    }
    Ok(())
}

/// The secret fields a credential, a login profile or a device's own
/// credentials carry. Required when creating; absent on a change, the sealed
/// ones stay.
#[derive(Deserialize)]
pub struct SecretInput {
    /// Kind `password`.
    pub(super) password: Option<SecretString>,
    /// Kind `ssh_key`: the key, with its passphrase and certificate if any.
    pub(super) private_key: Option<SecretString>,
    passphrase: Option<SecretString>,
    certificate: Option<String>,
}

impl SecretInput {
    /// Empty fields are ones left empty to keep what is stored.
    pub(super) fn given(mut self) -> Self {
        self.password = self.password.filter(|p| !p.expose_secret().is_empty());
        self.private_key = self
            .private_key
            .filter(|k| !k.expose_secret().trim().is_empty());
        self
    }
}

/// The secret fields of one version, checked and ready to seal.
pub(super) enum Secrets {
    Password(SecretString),
    Key {
        private_key: SecretString,
        passphrase: Option<SecretString>,
        certificate: Option<String>,
        algorithm: String,
        fingerprint: String,
    },
}

impl Secrets {
    fn fields(&self) -> Vec<(&'static str, &[u8])> {
        match self {
            Secrets::Password(password) => {
                vec![(PASSWORD_FIELD, password.expose_secret().as_bytes())]
            }
            Secrets::Key {
                private_key,
                passphrase,
                certificate,
                ..
            } => {
                let mut fields = vec![(PRIVATE_KEY_FIELD, private_key.expose_secret().as_bytes())];
                if let Some(passphrase) = passphrase {
                    fields.push((PASSPHRASE_FIELD, passphrase.expose_secret().as_bytes()));
                }
                if let Some(certificate) = certificate {
                    fields.push((CERTIFICATE_FIELD, certificate.as_bytes()));
                }
                fields
            }
        }
    }

    /// What identifies a key, stored in plain text to show it.
    pub(super) fn key_info(&self) -> (Option<&str>, Option<&str>, bool) {
        match self {
            Secrets::Password(_) => (None, None, false),
            Secrets::Key {
                algorithm,
                fingerprint,
                certificate,
                ..
            } => (Some(algorithm), Some(fingerprint), certificate.is_some()),
        }
    }
}

const PRIVATE_KEY_FIELD: &str = "private_key";
const PASSPHRASE_FIELD: &str = "passphrase";
const CERTIFICATE_FIELD: &str = "certificate";

/// The new secrets of `kind` (`password` or `ssh_key`), if any were given;
/// keys are parsed and matched against their certificate before anything is
/// stored.
pub(super) fn new_secrets(kind: &str, input: &SecretInput) -> Result<Option<Secrets>, Problem> {
    match kind {
        "password" => Ok(input.password.clone().map(Secrets::Password)),
        "ssh_key" => {
            let Some(private_key) = input.private_key.clone() else {
                return Ok(None);
            };
            let passphrase = input
                .passphrase
                .clone()
                .filter(|p| !p.expose_secret().is_empty());
            let certificate = input
                .certificate
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_owned);
            let key = SshKey::parse(
                private_key.expose_secret(),
                passphrase.as_ref().map(|p| p.expose_secret()),
                certificate.as_deref(),
            )
            .map_err(|error| {
                invalid(match error {
                    KeyError::InvalidKey => "private_key",
                    KeyError::PassphraseRequired | KeyError::WrongPassphrase => "passphrase",
                    KeyError::InvalidCertificate | KeyError::CertificateMismatch => "certificate",
                })
            })?;
            Ok(Some(Secrets::Key {
                private_key,
                passphrase,
                certificate,
                algorithm: key.algorithm(),
                fingerprint: key.fingerprint(),
            }))
        }
        _ => Err(invalid("secret_kind")),
    }
}

pub(super) fn plain(value: &str, field: &str) -> Result<String, Problem> {
    let value = value.trim();
    if value.chars().count() > 256 || value.chars().any(char::is_control) {
        return Err(invalid(field));
    }
    Ok(value.to_owned())
}

pub(super) async fn seal_version(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    id: Uuid,
    version: i32,
    secrets: &Secrets,
) -> Result<(), Problem> {
    for (field, value) in secrets.fields() {
        secrets::store(&mut *tx, &state.vault, id, version, field, value)
            .await
            .map_err(secret_problem)?;
    }
    Ok(())
}

async fn seal_totp(
    tx: &mut sqlx::PgConnection,
    state: &AppState,
    id: Uuid,
    version: i32,
    totp: &SecretString,
) -> Result<(), Problem> {
    secrets::store(
        &mut *tx,
        &state.vault,
        id,
        version,
        TOTP_FIELD,
        totp.expose_secret().as_bytes(),
    )
    .await
    .map_err(secret_problem)
}

pub async fn create_credential(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<CredentialInput>, JsonRejection>,
) -> Result<(StatusCode, Json<Created>), Problem> {
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let username = plain(&input.username, "username")?;
    let secrets = new_secrets("password", &input.secrets)?.ok_or_else(|| invalid("password"))?;
    let details = details(&input)?;
    let totp = totp_input(&input)?.flatten();
    // A new credential has no protected value to keep.
    if !details.fields.kept.is_empty() {
        return Err(invalid("fields"));
    }
    let (subject, catalog) = context(&state, &session).await?;
    require(
        &catalog,
        &subject,
        Role::Edit,
        ObjectId::Collection(input.collection_id),
    )?;

    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO credentials (collection_id, name, username, url, notes, icon, fields, tags,
                                  expires_on, has_totp)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::date, $10) RETURNING id",
    )
    .bind(input.collection_id)
    .bind(&name)
    .bind(&username)
    .bind(&details.url)
    .bind(&details.notes)
    .bind(details.icon)
    .bind(sqlx::types::Json(&details.fields.stored))
    .bind(&details.tags)
    .bind(&details.expires_on)
    .bind(totp.is_some())
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    seal_version(&mut tx, &state, id, 1, &secrets).await?;
    seal_fields(&mut tx, &state, id, 1, &details.fields.sealed).await?;
    if let Some(totp) = &totp {
        seal_totp(&mut tx, &state, id, 1, totp).await?;
    }
    let details = json!({
        "name": name, "username": username, "tags": details.tags,
        "expires_on": details.expires_on, "has_totp": totp.is_some(),
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CredentialCreated,
            ObjectId::Credential(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}

pub async fn update_credential(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<CredentialInput>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let username = plain(&input.username, "username")?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Credential(id))?;
    if catalog.parent(ObjectId::Credential(id)) != Some(input.collection_id) {
        require(
            &catalog,
            &subject,
            Role::Edit,
            ObjectId::Collection(input.collection_id),
        )?;
    }
    let (before, stored, had_totp): (i32, sqlx::types::Json<Vec<Field>>, bool) =
        sqlx::query_as("SELECT version, fields, has_totp FROM credentials WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let secrets = new_secrets("password", &input.secrets)?;
    let details = details(&input)?;
    let totp = totp_input(&input)?;
    if !details.fields.keeps_only_what_was(&stored) {
        return Err(invalid("fields"));
    }
    // Every secret belongs to a version: one that changes makes a new one,
    // and what did not change is copied into it.
    let fields_changed = details.fields.changes_secrets(&stored);
    let totp_changed = matches!(totp, Some(Some(_))) || (had_totp && matches!(totp, Some(None)));
    let changed = secrets.is_some() || fields_changed || totp_changed;
    let has_totp = match &totp {
        None => had_totp,
        Some(given) => given.is_some(),
    };

    let mut tx = state.db.begin().await?;
    let version: i32 = sqlx::query_scalar(
        "UPDATE credentials SET collection_id = $2, name = $3, username = $4,
             url = $6, notes = $7, icon = $8, fields = $9, tags = $10, expires_on = $11::date,
             has_totp = $12,
             version = version + CASE WHEN $5 THEN 1 ELSE 0 END, updated_at = now()
         WHERE id = $1 RETURNING version",
    )
    .bind(id)
    .bind(input.collection_id)
    .bind(&name)
    .bind(&username)
    .bind(changed)
    .bind(&details.url)
    .bind(&details.notes)
    .bind(details.icon)
    .bind(sqlx::types::Json(&details.fields.stored))
    .bind(&details.tags)
    .bind(&details.expires_on)
    .bind(has_totp)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    if changed {
        let mut kept: Vec<String> = details
            .fields
            .kept
            .iter()
            .map(|name| fields::secret_name(name))
            .collect();
        if secrets.is_none() {
            kept.push(PASSWORD_FIELD.to_owned());
        }
        if totp.is_none() && had_totp {
            kept.push(TOTP_FIELD.to_owned());
        }
        carry(&mut tx, &state, id, before, version, &kept).await?;
        seal_fields(&mut tx, &state, id, version, &details.fields.sealed).await?;
    }
    if let Some(secrets) = &secrets {
        seal_version(&mut tx, &state, id, version, secrets).await?;
    }
    if let Some(Some(totp)) = &totp {
        seal_totp(&mut tx, &state, id, version, totp).await?;
    }
    let details = json!({
        "name": name, "username": username, "tags": details.tags,
        "expires_on": details.expires_on, "has_totp": has_totp,
        "collection_id": input.collection_id, "secret_changed": secrets.is_some(),
        "fields_changed": fields_changed, "totp_changed": totp_changed,
    });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CredentialUpdated,
            ObjectId::Credential(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    purge: bool,
}

/// `DELETE /api/credentials/{id}`: moves a credential into the recycle bin
/// (#193); one already there stays there. With `?purge=true` it goes for
/// good, with its sealed secrets and files, wherever it lies.
pub async fn delete_credential(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    query: Result<Query<DeleteQuery>, QueryRejection>,
) -> Result<StatusCode, Problem> {
    let Query(query) = query.map_err(|_| Problem::new(ErrorCode::InvalidRequest))?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Credential(id))?;
    let mut tx = state.db.begin().await?;
    if query.purge {
        purge_credentials(&mut tx, &[id]).await?;
    } else {
        // A repeated request, a double click say, must not purge what the
        // first one binned.
        let binned = sqlx::query(
            "UPDATE credentials SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if binned == 0 {
            return Ok(StatusCode::NO_CONTENT);
        }
    }
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CredentialDeleted,
            ObjectId::Credential(id),
            json!({ "purged": query.purge }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes credentials for good, with their sealed secrets and files.
pub(super) async fn purge_credentials(
    tx: &mut sqlx::PgConnection,
    ids: &[Uuid],
) -> Result<(), Problem> {
    // The files' contents are sealed under their own IDs.
    sqlx::query(
        "DELETE FROM secret_fields
         WHERE owner_id IN (SELECT id FROM credential_attachments WHERE credential_id = ANY($1))",
    )
    .bind(ids)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = ANY($1)")
        .bind(ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM credentials WHERE id = ANY($1)")
        .bind(ids)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// `POST /api/credentials/{id}/restore`: takes a credential out of the
/// recycle bin, back into its collection.
pub async fn restore_credential(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Credential(id))?;
    let mut tx = state.db.begin().await?;
    let restored = sqlx::query(
        "UPDATE credentials SET deleted_at = NULL WHERE id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if restored == 0 {
        return Err(invalid("id"));
    }
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CredentialRestored,
            ObjectId::Credential(id),
            json!({}),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn secret_problem(error: secrets::SecretError) -> Problem {
    tracing::error!(%error, "cannot seal a secret");
    Problem::new(ErrorCode::Internal)
}

// ── Grants ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ObjectQuery {
    pub(super) kind: String,
    pub(super) id: Uuid,
}

pub(super) fn object(kind: &str, id: Uuid) -> Result<ObjectId, Problem> {
    match kind {
        "folder" => Ok(ObjectId::Folder(id)),
        "device" => Ok(ObjectId::Device(id)),
        "credential" => Ok(ObjectId::Credential(id)),
        "collection" => Ok(ObjectId::Collection(id)),
        _ => Err(invalid("kind")),
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct GrantRow {
    id: Uuid,
    folder_id: Option<Uuid>,
    device_id: Option<Uuid>,
    credential_id: Option<Uuid>,
    collection_id: Option<Uuid>,
    principal_kind: String,
    principal_sid: String,
    principal_name: String,
    role: String,
    /// RFC 3339 in UTC, for a just-in-time grant.
    expires_at: Option<String>,
}

#[derive(Serialize)]
pub struct Grants {
    /// Grants on the object itself.
    direct: Vec<GrantRow>,
    /// Grants on folders above it, which hold here too.
    inherited: Vec<GrantRow>,
}

pub async fn list_grants(
    State(state): State<AppState>,
    session: Session,
    query: Result<Query<ObjectQuery>, QueryRejection>,
) -> Result<Json<Grants>, Problem> {
    let Query(query) = query.map_err(|_| Problem::new(ErrorCode::InvalidRequest))?;
    let target = object(&query.kind, query.id)?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, target)?;

    // The folders above a folder or device, the collections above a
    // collection or credential (#190). Apart: a collection made from a
    // folder has the folder's id.
    let mut above = Vec::new();
    let mut next = catalog.container(target);
    while let Some(container) = next {
        if above.contains(&container) {
            break;
        }
        above.push(container);
        next = catalog.container(container);
    }
    let rows: Vec<GrantRow> = sqlx::query_as(
        "SELECT id, folder_id, device_id, credential_id, collection_id, principal_kind,
                principal_sid, principal_name, role,
                to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS expires_at
         FROM grants
         WHERE (folder_id = ANY($1) OR collection_id = ANY($1) OR folder_id = $2 OR device_id = $2
                OR credential_id = $2 OR collection_id = $2)
           AND (expires_at IS NULL OR expires_at > now())
         ORDER BY lower(principal_name)",
    )
    .bind(above.iter().map(|o| catalog::id(*o)).collect::<Vec<_>>())
    .bind(query.id)
    .fetch_all(&state.db)
    .await?;
    // The query casts a wide net; each row counts only for the object it
    // names, which must be the target or one of the containers above it.
    type Sorted = Vec<(bool, GrantRow)>;
    let (direct, inherited): (Sorted, Sorted) = rows
        .into_iter()
        .filter_map(|row| {
            let on = catalog::grant_object(
                row.folder_id,
                row.device_id,
                row.credential_id,
                row.collection_id,
            )?;
            (on == target || above.contains(&on)).then_some((on == target, row))
        })
        .partition(|(own, _)| *own);
    let direct = direct.into_iter().map(|(_, row)| row).collect();
    let inherited = inherited.into_iter().map(|(_, row)| row).collect();
    Ok(Json(Grants { direct, inherited }))
}

#[derive(Deserialize)]
pub struct NewGrant {
    object: ObjectQuery,
    principal_kind: String,
    principal_sid: String,
    principal_name: String,
    role: String,
    /// RFC 3339, when the grant ends by itself (#178); none: it stays.
    #[serde(default)]
    expires_at: Option<String>,
}

/// Grants a role; an existing grant without end for the same principal on
/// the same object is changed to the new role. A grant with an end is one
/// of its own beside it.
pub async fn add_grant(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewGrant>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let target = object(&input.object.kind, input.object.id)?;
    let role = Role::parse(&input.role).ok_or_else(|| invalid("role"))?;
    if !matches!(input.principal_kind.as_str(), "user" | "group") {
        return Err(invalid("principal_kind"));
    }
    let sid: PrincipalId = input
        .principal_sid
        .parse()
        .map_err(|_| invalid("principal_sid"))?;
    let principal_name = name(&input.principal_name, "principal_name")?;
    // Checked here, stored by the database from the same text.
    let rfc3339 = &time::format_description::well_known::Rfc3339;
    let expires_at = match input.expires_at.as_deref() {
        None => None,
        Some(text) => Some(
            time::OffsetDateTime::parse(text, rfc3339)
                .ok()
                .filter(|at| *at > time::OffsetDateTime::now_utc())
                .and_then(|at| at.format(rfc3339).ok())
                .ok_or_else(|| invalid("expires_at"))?,
        ),
    };
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, target)?;
    if !sid.check(&state.db, &input.principal_kind).await? {
        return Err(invalid("principal_sid"));
    }

    let [folder, device, credential, collection] =
        catalog::grant_columns(target).ok_or_else(|| invalid("kind"))?;
    let mut tx = state.db.begin().await?;
    // The conflict only ever arises without an end: the unique index covers
    // grants without one.
    sqlx::query(
        "INSERT INTO grants (folder_id, device_id, credential_id, collection_id, principal_kind,
                             principal_sid, principal_name, role, created_by, expires_at)
         VALUES ($1, $2, $3, $10, $4, $5, $6, $7, $8, $9::timestamptz)
         ON CONFLICT (folder_id, device_id, credential_id, collection_id, principal_sid)
             WHERE expires_at IS NULL
         DO UPDATE SET role = EXCLUDED.role, principal_name = EXCLUDED.principal_name",
    )
    .bind(folder)
    .bind(device)
    .bind(credential)
    .bind(&input.principal_kind)
    .bind(sid.to_string())
    .bind(&principal_name)
    .bind(role.as_str())
    .bind(session.user_id)
    .bind(expires_at)
    .bind(collection)
    .execute(&mut *tx)
    .await?;
    let details = json!({
        "principal_kind": input.principal_kind, "principal_sid": sid.to_string(),
        "principal_name": principal_name, "role": role, "expires_at": input.expires_at,
    });
    audit::record(
        &mut *tx,
        entry(&session, Action::GrantAdded, target, details, &address),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_grant(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let row: Option<GrantRow> = sqlx::query_as(
        "SELECT id, folder_id, device_id, credential_id, collection_id, principal_kind,
                principal_sid, principal_name, role,
                to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS expires_at
         FROM grants WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let row = row.ok_or(Problem::new(ErrorCode::NotFound))?;
    let target = catalog::grant_object(
        row.folder_id,
        row.device_id,
        row.credential_id,
        row.collection_id,
    )
    .ok_or(Problem::new(ErrorCode::Internal))?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, target)?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM grants WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let details = json!({
        "principal_kind": row.principal_kind, "principal_sid": row.principal_sid,
        "principal_name": row.principal_name, "role": row.role,
    });
    audit::record(
        &mut *tx,
        entry(&session, Action::GrantRemoved, target, details, &address),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Directory ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct Search {
    q: String,
}

/// Users and groups to grant access to — for anyone who may manage something.
pub async fn search_principals(
    State(state): State<AppState>,
    session: Session,
    query: Result<Query<Search>, QueryRejection>,
) -> Result<Json<Vec<Principal>>, Problem> {
    let Query(query) = query.map_err(|_| Problem::new(ErrorCode::InvalidRequest))?;
    let (subject, catalog) = context(&state, &session).await?;
    let manages_something = subject.admin
        || catalog
            .visible(&subject)
            .roles
            .values()
            .any(|role| *role == Role::Manage);
    if !manages_something {
        return Err(Problem::new(ErrorCode::Forbidden));
    }
    let mut found = principal::search_own(&state.db, &query.q, 25).await?;
    // Without a directory, remotehub's own principals are all there is.
    if let Some(directory) = state.directory.get() {
        let listed = directory.search(&query.q, 25).await.map_err(|error| {
            tracing::warn!(%error, "directory search failed");
            Problem::new(ErrorCode::DirectoryUnavailable)
        })?;
        found.extend(listed.into_iter().map(Principal::from));
    }
    Ok(Json(found))
}
