//! Files kept with a shared credential (#100), sealed in the vault like
//! its password.
//!
//! - `POST /api/credentials/{id}/attachments?name=…`: the body is the file;
//!   a file of the same name is replaced. Needs `edit`.
//! - `GET /api/credentials/{id}/attachments/{attachment}`: the file, with
//!   `reveal`, audited like every reveal.
//! - `DELETE /api/credentials/{id}/attachments/{attachment}`: needs `edit`.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use remotehub_model::{ObjectId, Role};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::catalog::{context, entry, invalid, require};
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action};
use crate::secrets;
use crate::session::Session;

/// The largest file, in bytes.
pub const MAX_SIZE: usize = 5 * 1024 * 1024;
const FIELD: &str = "content";

#[derive(Deserialize)]
pub struct Upload {
    name: String,
}

fn sealed(error: secrets::SecretError) -> Problem {
    tracing::error!(%error, "an attachment cannot be sealed or opened");
    Problem::new(ErrorCode::Internal)
}

/// A file name as people see it: no path, no control characters.
fn file_name(name: &str) -> Result<String, Problem> {
    let name = name.trim();
    let bad = name.is_empty()
        || name.chars().count() > 255
        || name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\');
    if bad {
        return Err(invalid("name"));
    }
    Ok(name.to_owned())
}

pub async fn upload(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path(id): Path<Uuid>,
    Query(upload): Query<Upload>,
    headers: HeaderMap,
    content: Bytes,
) -> Result<StatusCode, Problem> {
    let name = file_name(&upload.name)?;
    if content.len() > MAX_SIZE {
        return Err(invalid("size"));
    }
    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 255 && !value.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Credential(id))?;

    let mut tx = state.db.begin().await?;
    let replaced: Option<Uuid> = sqlx::query_scalar(
        "DELETE FROM credential_attachments WHERE credential_id = $1 AND name = $2 RETURNING id",
    )
    .bind(id)
    .bind(&name)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(old) = replaced {
        sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
            .bind(old)
            .execute(&mut *tx)
            .await?;
    }
    let attachment: Uuid = sqlx::query_scalar(
        "INSERT INTO credential_attachments (credential_id, name, media_type, size, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(id)
    .bind(&name)
    .bind(&media_type)
    .bind(i32::try_from(content.len()).unwrap_or(i32::MAX))
    .bind(session.user_id)
    .fetch_one(&mut *tx)
    .await?;
    secrets::store(&mut *tx, &state.vault, attachment, 1, FIELD, &content)
        .await
        .map_err(sealed)?;
    let details =
        json!({ "attachment": name, "size": content.len(), "replaced": replaced.is_some() });
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CredentialAttachmentAdded,
            ObjectId::Credential(id),
            details,
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn download(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path((id, attachment)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Reveal, ObjectId::Credential(id))?;
    let name: String = sqlx::query_scalar(
        "SELECT name FROM credential_attachments WHERE id = $1 AND credential_id = $2",
    )
    .bind(attachment)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))?;
    let content = secrets::load(&state.db, &state.vault, attachment, 1, FIELD)
        .await
        .map_err(sealed)?
        .ok_or(Problem::new(ErrorCode::Internal))?;
    audit::record(
        &state.db,
        entry(
            &session,
            Action::CredentialRevealed,
            ObjectId::Credential(id),
            json!({ "purpose": "download", "attachment": name }),
            &address,
        ),
    )
    .await?;
    // Never shown in the page's origin: always a download.
    let disposition = format!("attachment; filename*=UTF-8''{}", percent_encode(&name));
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        content.to_vec(),
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    Path((id, attachment)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Problem> {
    let (subject, catalog) = context(&state, &session).await?;
    require(&catalog, &subject, Role::Edit, ObjectId::Credential(id))?;
    let mut tx = state.db.begin().await?;
    let name: String = sqlx::query_scalar(
        "DELETE FROM credential_attachments WHERE id = $1 AND credential_id = $2 RETURNING name",
    )
    .bind(attachment)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Problem::new(ErrorCode::NotFound))?;
    sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
        .bind(attachment)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::CredentialAttachmentDeleted,
            ObjectId::Credential(id),
            json!({ "attachment": name }),
            &address,
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// RFC 5987 encoding for `filename*`: everything but unreserved characters
/// as `%XX` of its UTF-8 bytes.
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_names() {
        assert_eq!(file_name(" key.pem ").unwrap(), "key.pem");
        for bad in ["", "../etc/passwd", "a\\b", "a\nb", &"x".repeat(256)] {
            assert!(file_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn file_names_are_encoded_for_the_header() {
        assert_eq!(
            percent_encode("Zertifikat äö.pem"),
            "Zertifikat%20%C3%A4%C3%B6.pem"
        );
        assert_eq!(percent_encode("a\"b;c"), "a%22b%3Bc");
    }
}
