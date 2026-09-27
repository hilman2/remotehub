//! Loads both trees and all grants for `authorize()` (crates/model).

use remotehub_model::{Catalog, Grant, ObjectId, Role};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct GrantLink {
    folder_id: Option<Uuid>,
    device_id: Option<Uuid>,
    credential_id: Option<Uuid>,
    collection_id: Option<Uuid>,
    principal_sid: String,
    role: String,
    until: Option<i64>,
}

/// Folders with devices, collections with credentials (#190), and every
/// grant, as `authorize()` needs them. Loaded per request: small (thousands
/// of rows) and always current.
pub async fn load(db: &PgPool) -> Result<Catalog, sqlx::Error> {
    let folders: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as("SELECT id, parent_id FROM folders")
        .fetch_all(db)
        .await?;
    let collections: Vec<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT id, parent_id FROM collections")
            .fetch_all(db)
            .await?;
    let devices: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT id, folder_id FROM devices")
        .fetch_all(db)
        .await?;
    let credentials: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id, collection_id FROM credentials")
            .fetch_all(db)
            .await?;
    // The database's clock decides when a grant runs out, as it wrote
    // expires_at; running out is decided in the model.
    let (now,): (i64,) = sqlx::query_as("SELECT extract(epoch FROM now())::bigint")
        .fetch_one(db)
        .await?;
    let grants: Vec<GrantLink> = sqlx::query_as(
        "SELECT folder_id, device_id, credential_id, collection_id, principal_sid, role,
                extract(epoch FROM expires_at)::bigint AS until
         FROM grants",
    )
    .fetch_all(db)
    .await?;
    let grants = grants.into_iter().filter_map(|g| {
        Some(Grant {
            object: grant_object(g.folder_id, g.device_id, g.credential_id, g.collection_id)?,
            principal: g.principal_sid,
            role: Role::parse(&g.role)?,
            until: g.until,
        })
    });
    Ok(Catalog::new(
        folders,
        collections,
        devices,
        credentials,
        grants,
        now,
    ))
}

/// The object a row of requests points to (exactly one column is set).
pub fn object_id(
    folder: Option<Uuid>,
    device: Option<Uuid>,
    credential: Option<Uuid>,
) -> Option<ObjectId> {
    grant_object(folder, device, credential, None)
}

/// The object a grant row points to (exactly one column is set), a
/// collection included (#190).
pub fn grant_object(
    folder: Option<Uuid>,
    device: Option<Uuid>,
    credential: Option<Uuid>,
    collection: Option<Uuid>,
) -> Option<ObjectId> {
    match (folder, device, credential, collection) {
        (Some(id), None, None, None) => Some(ObjectId::Folder(id)),
        (None, Some(id), None, None) => Some(ObjectId::Device(id)),
        (None, None, Some(id), None) => Some(ObjectId::Credential(id)),
        (None, None, None, Some(id)) => Some(ObjectId::Collection(id)),
        _ => None,
    }
}

/// The name of an object's kind in the audit log and the API.
pub fn kind(object: ObjectId) -> &'static str {
    match object {
        ObjectId::Folder(_) => "folder",
        ObjectId::Device(_) => "device",
        ObjectId::Credential(_) => "credential",
        ObjectId::Collection(_) => "collection",
    }
}

pub fn id(object: ObjectId) -> Uuid {
    match object {
        ObjectId::Folder(id)
        | ObjectId::Device(id)
        | ObjectId::Credential(id)
        | ObjectId::Collection(id) => id,
    }
}

/// The grant table's columns for an object: folder, device, credential and
/// collection, exactly one set.
pub fn grant_columns(object: ObjectId) -> [Option<Uuid>; 4] {
    match object {
        ObjectId::Folder(id) => [Some(id), None, None, None],
        ObjectId::Device(id) => [None, Some(id), None, None],
        ObjectId::Credential(id) => [None, None, Some(id), None],
        ObjectId::Collection(id) => [None, None, None, Some(id)],
    }
}
