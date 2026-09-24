//! Folders, devices, credentials and grants (`/api/tree`, `/api/folders`,
//! `/api/devices`, `/api/credentials`, `/api/grants`, `/api/directory`).
//!
//! Every handler asks `authorize()` (crates/model) first and records what it
//! changed in the audit log, in the same transaction. Objects a user cannot
//! see answer `not_found`, so their existence does not leak.

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use remotehub_directory::{Principal, Sid};
use remotehub_model::{Catalog, ObjectId, Role, Subject};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::audit::{self, Action, Actor, Entry};
use crate::session::Session;
use crate::{AppState, catalog, secrets};

const PASSWORD_FIELD: &str = "password";

// ── Helpers ─────────────────────────────────────────────────────────────────

async fn context(state: &AppState, session: &Session) -> Result<(Subject, Catalog), Problem> {
    Ok((
        session.subject(&state.settings),
        catalog::load(&state.db).await?,
    ))
}

/// `not_found` for objects the subject cannot see at all, `forbidden` for
/// objects they see but may not act on in this way.
fn require(
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

fn body<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Problem> {
    body.map(|Json(value)| value)
        .map_err(|_| Problem::new(ErrorCode::InvalidRequest))
}

fn invalid(field: &str) -> Problem {
    Problem::new(ErrorCode::InvalidRequest).param("field", field)
}

/// A trimmed, non-empty name of at most 200 characters.
fn name(value: &str, field: &str) -> Result<String, Problem> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 200 || value.chars().any(char::is_control) {
        return Err(invalid(field));
    }
    Ok(value.to_owned())
}

/// Unique names and non-empty folders become their own problems.
fn database(error: sqlx::Error) -> Problem {
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

fn entry<'a>(
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
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

// ── Tree ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Tree {
    folders: Vec<TreeFolder>,
    devices: Vec<TreeDevice>,
    credentials: Vec<TreeCredential>,
    may_create_top_level: bool,
}

#[derive(Serialize)]
struct TreeFolder {
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
    credential_id: Option<Uuid>,
    description: String,
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

#[derive(Serialize, sqlx::FromRow)]
struct CredentialRow {
    id: Uuid,
    folder_id: Uuid,
    name: String,
    username: String,
    domain: String,
    version: i32,
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

    let folders: Vec<(Uuid, Option<Uuid>, String)> =
        sqlx::query_as("SELECT id, parent_id, name FROM folders ORDER BY lower(name)")
            .fetch_all(&state.db)
            .await?;
    let devices: Vec<DeviceRow> = sqlx::query_as(
        "SELECT id, folder_id, name, protocol, host, port, auth_mode, credential_id, description, host_key
         FROM devices ORDER BY lower(name)",
    )
    .fetch_all(&state.db)
    .await?;
    let credentials: Vec<CredentialRow> = sqlx::query_as(
        "SELECT id, folder_id, name, username, domain, version FROM credentials ORDER BY lower(name)",
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(Tree {
        folders: folders
            .into_iter()
            .filter_map(|(id, parent_id, name)| {
                let role = visible.roles.get(&ObjectId::Folder(id)).copied();
                (role.is_some() || visible.path_only.contains(&id)).then_some(TreeFolder {
                    id,
                    parent_id,
                    name,
                    role,
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
pub struct NewFolder {
    parent_id: Option<Uuid>,
    name: String,
}

#[derive(Serialize)]
pub struct Created {
    id: Uuid,
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

    let mut tx = state.db.begin().await?;
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO folders (parent_id, name) VALUES ($1, $2) RETURNING id")
            .bind(input.parent_id)
            .bind(&name)
            .fetch_one(&mut *tx)
            .await
            .map_err(database)?;
    let details = json!({ "name": name, "parent_id": input.parent_id });
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
}

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

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE folders SET
             name = coalesce($2, name),
             parent_id = CASE WHEN $3 THEN $4 ELSE parent_id END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(&name)
    .bind(input.parent_id.is_some())
    .bind(input.parent_id.flatten())
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    let details = json!({ "name": name, "parent_id": input.parent_id });
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
    credential_id: Option<Uuid>,
    #[serde(default)]
    description: String,
}

struct ValidDevice {
    folder_id: Uuid,
    name: String,
    protocol: &'static str,
    host: String,
    port: i32,
    auth_mode: &'static str,
    credential_id: Option<Uuid>,
    description: String,
}

impl DeviceInput {
    fn validate(self) -> Result<ValidDevice, Problem> {
        let protocol = match self.protocol.as_str() {
            "ssh" => "ssh",
            "rdp" => "rdp",
            "vnc" => "vnc",
            _ => return Err(invalid("protocol")),
        };
        let auth_mode = match self.auth_mode.as_str() {
            "stored" => "stored",
            "ask" => "ask",
            "own" => "own",
            _ => return Err(invalid("auth_mode")),
        };
        // A stored credential is exactly what "stored" means, and nothing else.
        if (auth_mode == "stored") != self.credential_id.is_some() {
            return Err(invalid("credential_id"));
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
        Ok(ValidDevice {
            folder_id: self.folder_id,
            name: name(&self.name, "name")?,
            protocol,
            host: host.to_owned(),
            port: i32::from(self.port),
            auth_mode,
            credential_id: self.credential_id,
            description: self.description.trim().to_owned(),
        })
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
    // Whoever points a stored credential at a host may use that credential:
    // otherwise any editor could send it to a host of their choosing.
    if let Some(credential) = device.credential_id {
        require(
            &catalog,
            &subject,
            Role::Connect,
            ObjectId::Credential(credential),
        )?;
    }

    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO devices (folder_id, name, protocol, host, port, auth_mode, credential_id, description)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(device.folder_id)
    .bind(&device.name)
    .bind(device.protocol)
    .bind(&device.host)
    .bind(device.port)
    .bind(device.auth_mode)
    .bind(device.credential_id)
    .bind(&device.description)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    let details = json!({
        "name": device.name, "protocol": device.protocol, "host": device.host, "port": device.port,
        "auth_mode": device.auth_mode, "credential_id": device.credential_id,
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
    let before: (String, String, i32, Option<Uuid>) =
        sqlx::query_as("SELECT protocol, host, port, credential_id FROM devices WHERE id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    // A credential may only be linked, or sent to a changed target, by
    // someone who may use it (see create_device).
    let target_changed =
        before.0 != device.protocol || before.1 != device.host || before.2 != device.port;
    if let Some(credential) = device.credential_id
        && (before.3 != Some(credential) || target_changed)
    {
        require(
            &catalog,
            &subject,
            Role::Connect,
            ObjectId::Credential(credential),
        )?;
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE devices SET folder_id = $2, name = $3, protocol = $4, host = $5, port = $6,
             auth_mode = $7, credential_id = $8, description = $9, updated_at = now(),
             host_key = CASE WHEN $10 THEN NULL ELSE host_key END,
             host_key_pinned_at = CASE WHEN $10 THEN NULL ELSE host_key_pinned_at END
         WHERE id = $1",
    )
    .bind(id)
    .bind(device.folder_id)
    .bind(&device.name)
    .bind(device.protocol)
    .bind(&device.host)
    .bind(device.port)
    .bind(device.auth_mode)
    .bind(device.credential_id)
    .bind(&device.description)
    .bind(target_changed)
    .execute(&mut *tx)
    .await
    .map_err(database)?;
    let details = json!({
        "name": device.name, "protocol": device.protocol, "host": device.host, "port": device.port,
        "auth_mode": device.auth_mode, "credential_id": device.credential_id, "folder_id": device.folder_id,
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
    let old: Option<String> = sqlx::query_scalar(
        "UPDATE devices d SET host_key = NULL, host_key_pinned_at = NULL
         FROM (SELECT host_key FROM devices WHERE id = $1) old
         WHERE d.id = $1 RETURNING old.host_key",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let details =
        json!({ "fingerprint": old.as_deref().and_then(remotehub_gateway::ssh::fingerprint) });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::HostKeyReset,
            ObjectId::Device(id),
            details,
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
    folder_id: Uuid,
    name: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    domain: String,
    /// Required when creating; when updating, absent keeps the password.
    password: Option<SecretString>,
}

fn plain(value: &str, field: &str) -> Result<String, Problem> {
    let value = value.trim();
    if value.chars().count() > 256 || value.chars().any(char::is_control) {
        return Err(invalid(field));
    }
    Ok(value.to_owned())
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
    let domain = plain(&input.domain, "domain")?;
    let password = input.password.ok_or_else(|| invalid("password"))?;
    let (subject, catalog) = context(&state, &session).await?;
    require(
        &catalog,
        &subject,
        Role::Edit,
        ObjectId::Folder(input.folder_id),
    )?;

    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO credentials (folder_id, name, username, domain) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(input.folder_id)
    .bind(&name)
    .bind(&username)
    .bind(&domain)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    secrets::store(
        &mut *tx,
        &state.vault,
        id,
        1,
        PASSWORD_FIELD,
        password.expose_secret().as_bytes(),
    )
    .await
    .map_err(secret_problem)?;
    let details = json!({ "name": name, "username": username, "domain": domain });
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
    let domain = plain(&input.domain, "domain")?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Credential(id))?;
    if catalog.parent(ObjectId::Credential(id)) != Some(input.folder_id) {
        require(
            &catalog,
            &subject,
            Role::Edit,
            ObjectId::Folder(input.folder_id),
        )?;
    }

    let mut tx = state.db.begin().await?;
    let version: i32 = sqlx::query_scalar(
        "UPDATE credentials SET folder_id = $2, name = $3, username = $4, domain = $5,
             version = version + CASE WHEN $6 THEN 1 ELSE 0 END, updated_at = now()
         WHERE id = $1 RETURNING version",
    )
    .bind(id)
    .bind(input.folder_id)
    .bind(&name)
    .bind(&username)
    .bind(&domain)
    .bind(input.password.is_some())
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    if let Some(password) = &input.password {
        secrets::store(
            &mut *tx,
            &state.vault,
            id,
            version,
            PASSWORD_FIELD,
            password.expose_secret().as_bytes(),
        )
        .await
        .map_err(secret_problem)?;
    }
    let details = json!({
        "name": name, "username": username, "domain": domain,
        "folder_id": input.folder_id, "password_changed": input.password.is_some(),
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

pub async fn delete_credential(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Credential(id))?;
    let mut tx = state.db.begin().await?;
    // Devices that used it fall back to asking for credentials.
    sqlx::query(
        "UPDATE devices SET auth_mode = 'ask', credential_id = NULL WHERE credential_id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM credentials WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CredentialDeleted,
            ObjectId::Credential(id),
            json!({}),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn secret_problem(error: secrets::SecretError) -> Problem {
    tracing::error!(%error, "cannot seal a secret");
    Problem::new(ErrorCode::Internal)
}

// ── Grants ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ObjectQuery {
    kind: String,
    id: Uuid,
}

fn object(kind: &str, id: Uuid) -> Result<ObjectId, Problem> {
    match kind {
        "folder" => Ok(ObjectId::Folder(id)),
        "device" => Ok(ObjectId::Device(id)),
        "credential" => Ok(ObjectId::Credential(id)),
        _ => Err(invalid("kind")),
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct GrantRow {
    id: Uuid,
    folder_id: Option<Uuid>,
    device_id: Option<Uuid>,
    credential_id: Option<Uuid>,
    principal_kind: String,
    principal_sid: String,
    principal_name: String,
    role: String,
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

    let mut ancestors = Vec::new();
    let mut next = catalog.parent(target);
    while let Some(folder) = next {
        if ancestors.contains(&folder) {
            break;
        }
        ancestors.push(folder);
        next = catalog.parent(ObjectId::Folder(folder));
    }
    let rows: Vec<GrantRow> = sqlx::query_as(
        "SELECT id, folder_id, device_id, credential_id, principal_kind, principal_sid, principal_name, role
         FROM grants
         WHERE folder_id = ANY($1) OR folder_id = $2 OR device_id = $2 OR credential_id = $2
         ORDER BY lower(principal_name)",
    )
    .bind(&ancestors)
    .bind(query.id)
    .fetch_all(&state.db)
    .await?;
    let (direct, inherited) = rows.into_iter().partition(|row| {
        catalog::object_id(row.folder_id, row.device_id, row.credential_id) == Some(target)
    });
    Ok(Json(Grants { direct, inherited }))
}

#[derive(Deserialize)]
pub struct NewGrant {
    object: ObjectQuery,
    principal_kind: String,
    principal_sid: String,
    principal_name: String,
    role: String,
}

/// Grants a role; an existing grant for the same principal on the same
/// object is changed to the new role.
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
    let sid: Sid = input
        .principal_sid
        .parse()
        .map_err(|_| invalid("principal_sid"))?;
    let principal_name = name(&input.principal_name, "principal_name")?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, target)?;

    let (folder, device, credential) = match target {
        ObjectId::Folder(id) => (Some(id), None, None),
        ObjectId::Device(id) => (None, Some(id), None),
        ObjectId::Credential(id) => (None, None, Some(id)),
    };
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO grants (folder_id, device_id, credential_id, principal_kind, principal_sid,
                             principal_name, role, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (folder_id, device_id, credential_id, principal_sid)
         DO UPDATE SET role = EXCLUDED.role, principal_name = EXCLUDED.principal_name",
    )
    .bind(folder)
    .bind(device)
    .bind(credential)
    .bind(&input.principal_kind)
    .bind(sid.as_str())
    .bind(&principal_name)
    .bind(role.as_str())
    .bind(session.user_id)
    .execute(&mut *tx)
    .await?;
    let details = json!({
        "principal_kind": input.principal_kind, "principal_sid": sid,
        "principal_name": principal_name, "role": role,
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
        "SELECT id, folder_id, device_id, credential_id, principal_kind, principal_sid, principal_name, role
         FROM grants WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let row = row.ok_or(Problem::new(ErrorCode::NotFound))?;
    let target = catalog::object_id(row.folder_id, row.device_id, row.credential_id)
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
    let directory = state
        .directory
        .as_ref()
        .ok_or(Problem::new(ErrorCode::DirectoryUnavailable))?;
    let found = directory.search(&query.q, 25).await.map_err(|error| {
        tracing::warn!(%error, "directory search failed");
        Problem::new(ErrorCode::DirectoryUnavailable)
    })?;
    Ok(Json(found))
}
