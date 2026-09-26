//! HTTPS for the web interface: the certificate, and the server that hands
//! each connection to the router.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::{Extension, Router};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::files;
use crate::ui::Peer;

/// A client gets this long for the TLS handshake and for each request's
/// headers; slow ones would otherwise hold connections open.
const HANDSHAKE: Duration = Duration::from_secs(10);

/// The acceptor for `certificate` (PEM certificate chain and key), or for a
/// self-signed certificate in `data`, made on the first start. Also returns
/// the certificate's SHA-256 fingerprint, which the connector logs so that
/// the first visit can compare it with the browser's warning.
pub fn acceptor(
    certificate: Option<&(std::path::PathBuf, std::path::PathBuf)>,
    data: &Path,
) -> anyhow::Result<(TlsAcceptor, String)> {
    let (cert_file, key_file) = match certificate {
        Some((cert, key)) => (cert.clone(), key.clone()),
        None => {
            let cert = data.join("ui-certificate.pem");
            let key = data.join("ui-key.pem");
            if !cert.exists() || !key.exists() {
                let made = rcgen::generate_simple_self_signed(vec![
                    "localhost".to_owned(),
                    "127.0.0.1".to_owned(),
                ])
                .context("making the web interface's certificate")?;
                files::write_private(&key, made.signing_key.serialize_pem().as_bytes())?;
                std::fs::write(&cert, made.cert.pem())?;
            }
            (cert, key)
        }
    };
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert_file)
        .and_then(|certs| certs.collect())
        .with_context(|| format!("reading {}", cert_file.display()))?;
    let key = PrivateKeyDer::from_pem_file(&key_file)
        .with_context(|| format!("reading {}", key_file.display()))?;
    let fingerprint = chain
        .first()
        .map(|leaf| {
            Sha256::digest(leaf.as_ref())
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(":")
        })
        .context("the certificate file holds no certificate")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .context("the certificate and its key")?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((TlsAcceptor::from(Arc::new(config)), fingerprint))
}

/// Serves `router` over HTTPS until the future is dropped.
pub async fn serve(listener: TcpListener, acceptor: TlsAcceptor, router: Router) {
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(%error, "cannot accept a connection");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let router = router.clone().layer(Extension(Peer(peer)));
        tokio::spawn(async move {
            let Ok(Ok(tls)) = tokio::time::timeout(HANDSHAKE, acceptor.accept(tcp)).await else {
                return;
            };
            let _ = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HANDSHAKE)
                .serve_connection(TokioIo::new(tls), TowerToHyperService::new(router))
                .await;
        });
    }
}
