//! Learning the certificate a device presents, so remotehub can pin it on
//! first use (like SSH host keys) instead of trusting a CA: RDP servers and
//! appliances mostly use self-signed certificates.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("not reachable: {0}")]
    Unreachable(std::io::Error),
    #[error("no answer in time")]
    Timeout,
    #[error("the server offers RDP only without TLS")]
    NoTls,
    #[error("not an RDP server: {0}")]
    Protocol(&'static str),
    #[error("TLS handshake failed: {0}")]
    Tls(std::io::Error),
}

/// The certificate of an HTTPS server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    /// SHA-256 over the certificate, as [`fingerprint`] writes it: what
    /// remotehub pins.
    pub fingerprint: String,
    /// Base64 SHA-256 over its public key (SubjectPublicKeyInfo): what
    /// Chromium's `--ignore-certificate-errors-spki-list` takes.
    pub spki: String,
}

/// The certificate the HTTPS server `name` presents, reached at
/// `host:port`: the device's own address, or a forward to it. `name` goes
/// into the handshake (SNI) as a browser sends it.
pub async fn https_certificate(
    name: &str,
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<Certificate, ProbeError> {
    tokio::time::timeout(timeout, async {
        let stream = TcpStream::connect((host, port))
            .await
            .map_err(ProbeError::Unreachable)?;
        let tls = handshake(name, stream).await?;
        let der = leaf(&tls)?;
        Ok(Certificate {
            fingerprint: fingerprint(der),
            spki: spki_hash(der).ok_or(ProbeError::Protocol("unreadable certificate"))?,
        })
    })
    .await
    .map_err(|_| ProbeError::Timeout)?
}

/// A TLS handshake on `stream` that accepts any certificate.
pub(crate) async fn handshake(
    host: &str,
    stream: TcpStream,
) -> Result<TlsStream<TcpStream>, ProbeError> {
    let connector = TlsConnector::from(Arc::new(client_config()));
    let name = ServerName::try_from(host.to_owned())
        .unwrap_or_else(|_| ServerName::try_from("device.invalid").expect("a valid DNS name"));
    connector
        .connect(name, stream)
        .await
        .map_err(ProbeError::Tls)
}

/// The server's own certificate, first in its chain.
pub(crate) fn leaf(tls: &TlsStream<TcpStream>) -> Result<&[u8], ProbeError> {
    tls.get_ref()
        .1
        .peer_certificates()
        .and_then(|chain| chain.first())
        .map(AsRef::as_ref)
        .ok_or(ProbeError::Protocol("no certificate"))
}

/// SHA-256 over the DER certificate, upper-case hex pairs joined by `:`
/// (the form FreeRDP compares, after `sha256:`).
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Base64 SHA-256 over the certificate's SubjectPublicKeyInfo.
pub fn spki_hash(der: &[u8]) -> Option<String> {
    let (_, certificate) = x509_parser::parse_x509_certificate(der).ok()?;
    let digest = Sha256::digest(certificate.public_key().raw);
    Some(base64::engine::general_purpose::STANDARD.encode(digest))
}

fn client_config() -> ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Capture(provider)))
        .with_no_client_auth()
}

/// Accepts any certificate — trust comes from the pinned fingerprint, not
/// from a CA — but still checks that the server holds its key.
#[derive(Debug)]
struct Capture(Arc<CryptoProvider>);

impl ServerCertVerifier for Capture {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_look_like_freerdps() {
        assert_eq!(
            fingerprint(b"abc"),
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
    }

    #[test]
    fn the_key_hash_is_chromiums() {
        // The lab's LDAP certificate; the hash is what
        // `openssl x509 -pubkey | openssl pkey -pubin -outform der |
        // openssl dgst -sha256 -binary | base64` prints for it.
        let pem = include_str!("../../../deploy/testlab/dc/tls/ca.crt");
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap();
        assert_eq!(spki_hash(&der).as_deref(), Some(EXPECTED_CA_SPKI));
        assert_eq!(spki_hash(b"not a certificate"), None);
    }

    const EXPECTED_CA_SPKI: &str = "feGH82LSCi6N9dQklYBxzaPxTwhwkJa7nf2VPd+FMqY=";
}
