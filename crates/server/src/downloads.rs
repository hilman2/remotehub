//! Programs remotehub serves itself under `/downloads/`, without sign-in,
//! like the root certificate under `/ca.crt`: the site connector for Windows
//! (#188), since the customer's server reaches remotehub but perhaps not
//! GitHub, and the browser extension (#201), which has no store.
//!
//! The production image carries the files of its release in the directory
//! `REMOTEHUB_DOWNLOADS`; remotehub reads them at start:
//!
//! - `remotehub-connector.exe`
//! - `remotehub-extension.zip`: the extension to load unpacked, by hand
//! - `remotehub-extension.crx`: the same, signed, for browsers that install
//!   it by policy
//! - `remotehub-extension.json`: `{"id", "version"}` of the extension, from
//!   the build that signed it (web/extension/pack.mjs)
//!
//! `SHA256SUMS` lists the files served. `remotehub-extension.xml` is the
//! update manifest the policy `ExtensionInstallForcelist` points browsers
//! to; it names the CRX under the public origin.

use std::io;
use std::path::Path;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use data_encoding::HEXLOWER;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::AppState;

/// The site connector's name, in the directory and in the URL.
pub const PROGRAM: &str = "remotehub-connector.exe";
pub const EXTENSION_ZIP: &str = "remotehub-extension.zip";
pub const EXTENSION_CRX: &str = "remotehub-extension.crx";
const EXTENSION_INFO: &str = "remotehub-extension.json";
pub const UPDATE_MANIFEST: &str = "remotehub-extension.xml";
const SUMS: &str = "SHA256SUMS";

/// The files to serve and their hash lines, held in memory: a few MiB.
pub struct Downloads {
    files: Vec<Served>,
    sums: String,
    extension: Option<Extension>,
}

struct Served {
    name: &'static str,
    content_type: &'static str,
    bytes: Bytes,
}

/// The browser extension of this release, as the build that signed it
/// describes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Extension {
    /// Chromium's ID: derived from the key that signs the CRX, and the same
    /// for the unpacked ZIP, whose manifest carries the public key.
    pub id: String,
    pub version: String,
}

/// The hash lines only: the files' bytes are of no use in a log.
impl std::fmt::Debug for Downloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Downloads")
            .field("sums", &self.sums)
            .field("extension", &self.extension)
            .finish_non_exhaustive()
    }
}

/// The ID of the extension signed with the public development key
/// (web/extension-dev-key.pem). An image built without the maintainer's key
/// serves it; anyone can sign an extension with that ID.
pub const DEVELOPMENT_EXTENSION: &str = "obnekonmlefgdhgodgbjapgoophnhlao";

/// Whether `id` has the form of a Chromium extension ID: 32 letters from
/// `a` to `p`, the hex digits of a key's hash shifted into letters.
pub fn is_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// `1.2.3`: what Chromium takes as an extension's version, and what may go
/// into the update manifest unescaped.
fn is_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    (1..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
}

impl Downloads {
    /// The files in `dir`; none if it holds none of them, as in development.
    pub fn load(dir: &Path) -> io::Result<Option<Downloads>> {
        let read = |name: &str| match std::fs::read(dir.join(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        };
        let extension = match read(EXTENSION_INFO)? {
            Some(bytes) => {
                let extension: Extension = serde_json::from_slice(&bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                if !is_extension_id(&extension.id) || !is_version(&extension.version) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{EXTENSION_INFO} names no valid ID and version"),
                    ));
                }
                Some(extension)
            }
            None => None,
        };
        let mut files = Vec::new();
        for (name, content_type) in [
            (PROGRAM, "application/vnd.microsoft.portable-executable"),
            (EXTENSION_ZIP, "application/zip"),
            (EXTENSION_CRX, "application/x-chrome-extension"),
        ] {
            // A CRX without its description could not be named in the
            // update manifest; serving it alone would only confuse.
            if name == EXTENSION_CRX && extension.is_none() {
                continue;
            }
            if let Some(bytes) = read(name)? {
                files.push(Served {
                    name,
                    content_type,
                    bytes: Bytes::from(bytes),
                });
            }
        }
        if files.is_empty() {
            return Ok(None);
        }
        // The format of sha256sum and of the release's file, so
        // `Select-String` and `sha256sum -c` read it alike.
        let sums = files
            .iter()
            .map(|f| {
                format!(
                    "{}  {}\n",
                    HEXLOWER.encode(&Sha256::digest(&f.bytes)),
                    f.name
                )
            })
            .collect();
        let has_crx = files.iter().any(|f| f.name == EXTENSION_CRX);
        Ok(Some(Downloads {
            files,
            sums,
            extension: extension.filter(|_| has_crx),
        }))
    }

    /// The extension served for installation by policy, if there is one.
    pub fn extension(&self) -> Option<&Extension> {
        self.extension.as_ref()
    }
}

/// The update manifest for Chromium's policy `ExtensionInstallForcelist`:
/// the browser reads it without cookies and installs or updates the CRX it
/// names.
fn update_manifest(extension: &Extension, public_origin: &str) -> String {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n\
         <gupdate xmlns='http://www.google.com/update2/response' protocol='2.0'>\n  \
         <app appid='{id}'>\n    \
         <updatecheck codebase='{public_origin}/downloads/{EXTENSION_CRX}' version='{version}' />\n  \
         </app>\n\
         </gupdate>\n",
        id = extension.id,
        version = extension.version,
    )
}

/// `GET /downloads/{file}`: a file, the hash lines or the update manifest;
/// 404 for anything else, and while remotehub has nothing to serve.
pub async fn file(State(state): State<AppState>, UrlPath(name): UrlPath<String>) -> Response {
    let Some(downloads) = state.settings.downloads.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if name == SUMS {
        return (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            downloads.sums.clone(),
        )
            .into_response();
    }
    if name == UPDATE_MANIFEST {
        return match downloads.extension() {
            Some(extension) => (
                [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
                update_manifest(extension, &state.settings.public_origin),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    match downloads.files.iter().find(|f| f.name == name) {
        Some(served) => (
            [
                (header::CONTENT_TYPE, served.content_type.to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", served.name),
                ),
            ],
            served.bytes.clone(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
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
        assert!(downloads.extension().is_none());
    }

    #[test]
    fn the_extension_needs_its_description_to_be_installed_by_policy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(EXTENSION_ZIP), b"PK").unwrap();
        std::fs::write(dir.path().join(EXTENSION_CRX), b"Cr24").unwrap();
        let downloads = Downloads::load(dir.path()).unwrap().unwrap();
        assert!(downloads.extension().is_none());
        assert!(
            !downloads.sums.contains(EXTENSION_CRX),
            "{}",
            downloads.sums
        );
        assert!(downloads.sums.contains(EXTENSION_ZIP), "{}", downloads.sums);

        let info = dir.path().join(EXTENSION_INFO);
        std::fs::write(
            &info,
            r#"{"id": "abcdefghijklmnopabcdefghijklmnop", "version": "0.3.0"}"#,
        )
        .unwrap();
        let downloads = Downloads::load(dir.path()).unwrap().unwrap();
        assert_eq!(
            downloads.extension(),
            Some(&Extension {
                id: "abcdefghijklmnopabcdefghijklmnop".to_owned(),
                version: "0.3.0".to_owned()
            })
        );
        assert!(downloads.sums.contains(EXTENSION_CRX), "{}", downloads.sums);

        for wrong in [
            r#"{"id": "abc", "version": "0.3.0"}"#,
            r#"{"id": "abcdefghijklmnopabcdefghijklmnop", "version": "0.3.0' x='"}"#,
            "not json",
        ] {
            std::fs::write(&info, wrong).unwrap();
            assert!(Downloads::load(dir.path()).is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_update_manifest_names_the_crx_under_the_public_origin() {
        let extension = Extension {
            id: "abcdefghijklmnopabcdefghijklmnop".to_owned(),
            version: "1.2.3".to_owned(),
        };
        let xml = update_manifest(&extension, "https://remotehub.example.com");
        assert!(
            xml.contains("<app appid='abcdefghijklmnopabcdefghijklmnop'>"),
            "{xml}"
        );
        assert!(
            xml.contains(
                "codebase='https://remotehub.example.com/downloads/remotehub-extension.crx' version='1.2.3'"
            ),
            "{xml}"
        );
    }

    #[test]
    fn versions_are_numbers_and_dots() {
        for good in ["1", "0.3.0", "1.2.3.4", "10.20.30"] {
            assert!(is_version(good), "{good}");
        }
        for bad in ["", "1.", ".1", "1.2.3.4.5", "1.2-beta", "1.2.3'"] {
            assert!(!is_version(bad), "{bad}");
        }
    }
}
