//! Collections of shared credentials (#190): a tree of their own, apart
//! from the device folders, with grants inherited downwards like folders.
//!
//! - `POST /api/collections`: create, at the top level (administrators) or
//!   in a collection one manages
//! - `PATCH /api/collections/{id}`: rename or move
//! - `DELETE /api/collections/{id}`: remove an empty collection
//!
//! The tree itself comes with `GET /api/tree`; grants go through
//! `/api/grants` with the kind `collection`.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use remotehub_model::{ObjectId, Role};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::catalog::{
    Created, body, context, database, entry, invalid, name, nullable, purge_credentials, require,
};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action};
use crate::session::Session;

#[derive(Deserialize)]
pub struct NewCollection {
    parent_id: Option<Uuid>,
    name: String,
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<NewCollection>, JsonRejection>,
) -> Result<(StatusCode, Json<Created>), Problem> {
    let input = body(input)?;
    let name = name(&input.name, "name")?;
    let (subject, catalog) = context(&state, &session).await?;
    match input.parent_id {
        None if catalog.may_create_top_level(&subject) => {}
        None => return Err(Problem::new(ErrorCode::Forbidden)),
        Some(parent) => require(
            &catalog,
            &subject,
            Role::Manage,
            ObjectId::Collection(parent),
        )?,
    }
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO collections (parent_id, name) VALUES ($1, $2) RETURNING id",
    )
    .bind(input.parent_id)
    .bind(&name)
    .fetch_one(&mut *tx)
    .await
    .map_err(database)?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CollectionCreated,
            ObjectId::Collection(id),
            json!({ "name": name, "parent_id": input.parent_id }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Created::new(id))))
}

#[derive(Deserialize)]
pub struct CollectionChange {
    name: Option<String>,
    /// Absent: stays; `null`: to the top level; an id: into that collection.
    #[serde(default, deserialize_with = "nullable")]
    parent_id: Option<Option<Uuid>>,
}

pub async fn update(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    input: Result<Json<CollectionChange>, JsonRejection>,
) -> Result<StatusCode, Problem> {
    let input = body(input)?;
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, ObjectId::Collection(id))?;
    let name = input.name.as_deref().map(|n| name(n, "name")).transpose()?;
    if let Some(target) = input.parent_id {
        match target {
            None if catalog.may_create_top_level(&subject) => {}
            None => return Err(Problem::new(ErrorCode::Forbidden)),
            Some(parent) => {
                require(
                    &catalog,
                    &subject,
                    Role::Manage,
                    ObjectId::Collection(parent),
                )?;
                if catalog.is_within_collection(parent, id) {
                    return Err(invalid("parent_id"));
                }
            }
        }
    }
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE collections SET
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
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CollectionUpdated,
            ObjectId::Collection(id),
            json!({ "name": name, "parent_id": input.parent_id }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Only an empty collection goes: credentials and collections inside keep
/// it, as `folder_not_empty`. What lies in its recycle bin goes with it, as
/// the recycle bin empties (#193); the audit entry names how many.
pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Manage, ObjectId::Collection(id))?;
    let mut tx = state.db.begin().await?;
    let binned: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM credentials WHERE collection_id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    purge_credentials(&mut tx, &binned).await?;
    sqlx::query("DELETE FROM collections WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CollectionDeleted,
            ObjectId::Collection(id),
            json!({ "purged": binned.len() }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
