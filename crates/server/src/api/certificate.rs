//! `/api/settings/certificate` (#146): the certificate Caddy of the ops
//! package serves, for administrators; and `/ca.crt`, `/ca.cer`: the root
//! certificate of Caddy's own CA, for anyone, to hand out to the clients.
//!
//! - `GET`: whether Caddy of the package runs, the certificate it serves and
//!   where that comes from, and a certificate of your own if one is set.
//! - `PUT`: a certificate of your own, as PEM or PFX. It is checked against
//!   the host first; Caddy serves it without a restart.
//! - `DELETE`: back to Let's Encrypt and Caddy's own CA.
//!
//! Behind a reverse proxy of your own, that proxy holds the certificate,
//! and there is nothing to set here. Changes are audited.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::catalog::body;
use super::problem::{ErrorCode, Problem};
use super::session::ClientAddress;
use crate::AppState;
use crate::audit::{self, Action, Actor, Entry};
use crate::caddy::{Caddy, CaddyError, Tls};
use crate::certificate::{self, Info, Refusal};
use crate::session::Session;

fn require_admin(session: &Session) -> Result<(), Problem> {
    if session.is_admin() {
        Ok(())
    } else {
        Err(Problem::new(ErrorCode::Forbidden))
    }
}

/// Caddy of the package, if it runs.
fn caddy(state: &AppState) -> Option<&Caddy> {
    state.settings.caddy.as_deref().filter(|caddy| caddy.runs())
}

fn not_running() -> Problem {
    Problem::new(ErrorCode::CaddyUnavailable)
        .param("detail", "Caddy of the ops package does not run")
}

fn caddy_failed(error: CaddyError) -> Problem {
    tracing::warn!(%error, "Caddy");
    match error {
        CaddyError::Refused(detail) => {
            Problem::new(ErrorCode::CaddyRefused).param("detail", detail)
        }
        other => Problem::new(ErrorCode::CaddyUnavailable).param("detail", other.to_string()),
    }
}

/// Where the certificate Caddy serves comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    LetsEncrypt,
    /// Caddy's own CA, named after the host.
    Remotehub,
    /// The certificate of your own set here.
    Own,
    Other,
}

fn source(served: &Info, own: Option<&Info>, host: &str) -> Source {
    if own.is_some_and(|own| own.fingerprint == served.fingerprint) {
        Source::Own
    } else if served
        .issuer
        .contains(&format!("remotehub {host} Intermediate"))
    {
        Source::Remotehub
    } else if served.issuer.contains("Let's Encrypt") {
        Source::LetsEncrypt
    } else {
        Source::Other
    }
}

#[derive(Serialize)]
pub struct OwnCertificate {
    info: Info,
    chain_complete: bool,
}

#[derive(Serialize)]
pub struct Status {
    /// The host a certificate has to cover.
    host: String,
    /// Caddy of the package runs; otherwise a proxy of your own holds the
    /// certificate.
    runs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    served: Option<Info>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<Source>,
    #[serde(skip_serializing_if = "Option::is_none")]
    own: Option<OwnCertificate>,
    /// The fingerprint of Caddy's root, to compare with the one handed out.
    #[serde(skip_serializing_if = "Option::is_none")]
    root_fingerprint: Option<String>,
}

pub async fn status(
    State(state): State<AppState>,
    session: Session,
) -> Result<Json<Status>, Problem> {
    require_admin(&session)?;
    let host = state.settings.host();
    let Some(caddy) = caddy(&state) else {
        return Ok(Json(Status {
            host,
            runs: false,
            served: None,
            source: None,
            own: None,
            root_fingerprint: None,
        }));
    };
    let own = certificate::stored(&state.db, &state.vault)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "cannot read the stored certificate");
            Problem::new(ErrorCode::Internal)
        })?
        .and_then(|uploaded| {
            certificate::check(&uploaded, &host, now())
                .ok()
                .or_else(|| {
                    // Stored but run out: shown as it is, the page says so.
                    certificate::info(&uploaded.chain[0]).map(|info| certificate::Checked {
                        info,
                        chain_complete: uploaded.chain.len() > 1,
                    })
                })
        })
        .map(|checked| OwnCertificate {
            info: checked.info,
            chain_complete: checked.chain_complete,
        });
    let served = caddy
        .current()
        .await
        .ok()
        .and_then(|chain| chain.first().and_then(|c| certificate::info(c)));
    let source = served
        .as_ref()
        .map(|served| source(served, own.as_ref().map(|o| &o.info), &host));
    let root_fingerprint = caddy.root().await.ok().and_then(|pem| {
        CertificateDer::from_pem_slice(pem.as_bytes())
            .ok()
            .map(|der| certificate::fingerprint(&der))
    });
    Ok(Json(Status {
        host,
        runs: true,
        served,
        source,
        own,
        root_fingerprint,
    }))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Deserialize)]
pub struct Input {
    /// PEM: the chain, the server's certificate first.
    #[serde(default)]
    certificate: Option<String>,
    /// PEM: its key.
    #[serde(default)]
    key: Option<String>,
    /// A PFX file, in base64.
    #[serde(default)]
    pfx: Option<String>,
    #[serde(default)]
    password: String,
}

fn refused(refusal: Refusal) -> Problem {
    Problem::new(ErrorCode::CertificateRefused).param("reason", json!(refusal))
}

fn entry<'a>(
    session: &'a Session,
    action: Action,
    details: serde_json::Value,
    address: &'a str,
) -> Entry<'a> {
    Entry {
        actor: Actor {
            id: Some(session.user_id),
            name: &session.username,
        },
        action,
        object: None,
        details,
        address: Some(address),
    }
}

pub async fn upload(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
    input: Result<Json<Input>, JsonRejection>,
) -> Result<Json<OwnCertificate>, Problem> {
    require_admin(&session)?;
    let input = body(input)?;
    let uploaded = match (input.pfx, input.certificate, input.key) {
        (Some(pfx), _, _) => {
            let data = base64::engine::general_purpose::STANDARD
                .decode(pfx.trim())
                .map_err(|_| refused(Refusal::Unreadable))?;
            certificate::from_pfx(&data, &input.password)
        }
        (None, Some(chain), Some(key)) => certificate::from_pem(&chain, &key),
        _ => Err(Refusal::Unreadable),
    }
    .map_err(refused)?;
    let checked = certificate::check(&uploaded, &state.settings.host(), now()).map_err(refused)?;
    let caddy = caddy(&state).ok_or_else(not_running)?;

    // Stored and audited only once Caddy took it: the transaction ends
    // after Caddy's answer.
    let mut tx = state.db.begin().await?;
    certificate::store(
        &mut tx,
        &state.vault,
        &uploaded,
        checked.info.not_after,
        session.user_id,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "cannot store the certificate");
        Problem::new(ErrorCode::Internal)
    })?;
    audit::record(
        &mut *tx,
        entry(
            &session,
            Action::TlsCertificateUploaded,
            json!({
                "subject": checked.info.subject,
                "issuer": checked.info.issuer,
                "fingerprint": checked.info.fingerprint,
                "not_after": checked.info.not_after,
            }),
            &address,
        ),
    )
    .await?;
    let (chain, key) = certificate::to_pem(&uploaded);
    caddy
        .serve(&Tls::Own { chain, key })
        .await
        .map_err(caddy_failed)?;
    tx.commit().await?;
    Ok(Json(OwnCertificate {
        info: checked.info,
        chain_complete: checked.chain_complete,
    }))
}

pub async fn reset(
    State(state): State<AppState>,
    session: Session,
    ClientAddress(address): ClientAddress,
) -> Result<StatusCode, Problem> {
    require_admin(&session)?;
    let caddy = caddy(&state).ok_or_else(not_running)?;
    let mut tx = state.db.begin().await?;
    certificate::remove(&mut tx).await?;
    audit::record(
        &mut *tx,
        entry(&session, Action::TlsCertificateReset, json!({}), &address),
    )
    .await?;
    caddy.serve(&Tls::Automatic).await.map_err(caddy_failed)?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `/ca.crt`: the root certificate of Caddy's own CA, as PEM. Public like
/// any CA certificate: clients fetch it to trust remotehub.
pub async fn root_pem(state: State<AppState>) -> Response {
    root(state, false).await
}

/// `/ca.cer`: the same as DER, which Windows' certificate tools expect.
pub async fn root_der(state: State<AppState>) -> Response {
    root(state, true).await
}

async fn root(State(state): State<AppState>, der: bool) -> Response {
    let Some(caddy) = caddy(&state) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let pem = match caddy.root().await {
        Ok(pem) => pem,
        Err(error) => {
            tracing::warn!(%error, "cannot read the root certificate from Caddy");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let host = state.settings.host();
    if der {
        let Ok(certificate) = CertificateDer::from_pem_slice(pem.as_bytes()) else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        (
            [
                (header::CONTENT_TYPE, "application/pkix-cert".to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"remotehub-{host}-ca.cer\""),
                ),
            ],
            certificate.to_vec(),
        )
            .into_response()
    } else {
        (
            [
                (header::CONTENT_TYPE, "application/x-pem-file".to_owned()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"remotehub-{host}-ca.crt\""),
                ),
            ],
            pem,
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(issuer: &str, fingerprint: &str) -> Info {
        Info {
            subject: "CN=remotehub.example.com".into(),
            issuer: issuer.into(),
            names: vec!["remotehub.example.com".into()],
            not_before: 0,
            not_after: 0,
            fingerprint: fingerprint.into(),
        }
    }

    #[test]
    fn the_source_comes_from_the_issuer_or_the_own_certificate() {
        let host = "remotehub.example.com";
        let local = info("CN=remotehub remotehub.example.com Intermediate", "AA");
        assert_eq!(source(&local, None, host), Source::Remotehub);
        let le = info("C=US, O=Let's Encrypt, CN=R11", "BB");
        assert_eq!(source(&le, None, host), Source::LetsEncrypt);
        let company = info("CN=Example Issuing CA", "CC");
        assert_eq!(source(&company, None, host), Source::Other);
        assert_eq!(source(&company, Some(&company), host), Source::Own);
        // Caddy's CA for another host is not this one's.
        let elsewhere = info("CN=remotehub other.example.com Intermediate", "DD");
        assert_eq!(source(&elsewhere, None, host), Source::Other);
    }
}
