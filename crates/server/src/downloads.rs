//! The site connector for Windows, served by remotehub itself (#188): the
//! customer's server reaches remotehub anyway, GitHub perhaps not. The
//! production image carries `remotehub-connector.exe` of its release in the
//! directory `REMOTEHUB_CONNECTOR_DOWNLOADS`; remotehub reads it at start and
//! serves it and its `SHA256SUMS` under `/downloads/`, without sign-in, like
//! the root certificate under `/ca.crt`.

use std::io;
use std::path::Path;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use data_encoding::HEXLOWER;
use sha2::{Digest, Sha256};

use crate::AppState;

/// The program's name, in the directory and in the URL.
pub const PROGRAM: &str = "remotehub-connector.exe";
const SUMS: &str = "SHA256SUMS";

/// The program and its hash line, held in memory: a few MiB.
pub struct Downloads {
    program: Bytes,
    sums: String,
}

/// The hash line only: the program's bytes are of no use in a log.
impl std::fmt::Debug for Downloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Downloads")
            .field("sums", &self.sums)
            .finish_non_exhaustive()
    }
}

impl Downloads {
    /// The program in `dir`; none if there is none, as in development.
    pub fn load(dir: &Path) -> io::Result<Option<Downloads>> {
        let program = match std::fs::read(dir.join(PROGRAM)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        // The format of sha256sum and of the release's file, so
        // `Select-String` and `sha256sum -c` read it alike.
        let sums = format!(
            "{}  {PROGRAM}\n",
            HEXLOWER.encode(&Sha256::digest(&program))
        );
        Ok(Some(Downloads {
            program: Bytes::from(program),
            sums,
        }))
    }
}

/// `GET /downloads/{file}`: the program or its hash line; 404 for anything
/// else, and while remotehub has no program to serve.
pub async fn file(State(state): State<AppState>, UrlPath(name): UrlPath<String>) -> Response {
    let Some(downloads) = state.settings.downloads.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match name.as_str() {
        PROGRAM => (
            [
                (
                    header::CONTENT_TYPE,
                    "application/vnd.microsoft.portable-executable",
                ),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"remotehub-connector.exe\"",
                ),
            ],
            downloads.program.clone(),
        )
            .into_response(),
        SUMS => (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            downloads.sums.clone(),
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_line_is_that_of_sha256sum() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Downloads::load(dir.path()).unwrap().is_none());
        std::fs::write(dir.path().join(PROGRAM), b"MZ").unwrap();
        let downloads = Downloads::load(dir.path()).unwrap().unwrap();
        // printf MZ | sha256sum
        assert_eq!(
            downloads.sums,
            "9b8db510ef42b8ed54a3712636fda55a4f8cfcd5493e20b74ab00cd4f3979f2d  remotehub-connector.exe\n"
        );
    }
}
