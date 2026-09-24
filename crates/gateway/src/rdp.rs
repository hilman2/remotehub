//! The certificate of an RDP server, learned by remotehub itself so it can be
//! pinned on first use (like SSH host keys) and handed to guacd, whose
//! FreeRDP then accepts nothing else (`cert-fingerprints`).
//!
//! RDP starts in plain TCP: the client's X.224 connection request asks for
//! TLS or NLA (both run inside TLS), the server confirms, and the TLS
//! handshake follows on the same connection. The probe stops after the
//! handshake — it never signs in.

use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// TLS (`PROTOCOL_SSL`), NLA (`PROTOCOL_HYBRID`) and NLA with early user
/// authorisation (`PROTOCOL_HYBRID_EX`): everything that runs inside TLS.
const REQUESTED_PROTOCOLS: u32 = 0x01 | 0x02 | 0x08;

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

/// The SHA-256 fingerprint of the certificate the RDP server at `host:port`
/// presents, as `AA:BB:…` (the form FreeRDP compares, after `sha256:`).
pub async fn certificate_fingerprint(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<String, ProbeError> {
    tokio::time::timeout(timeout, probe(host, port))
        .await
        .map_err(|_| ProbeError::Timeout)?
}

async fn probe(host: &str, port: u16) -> Result<String, ProbeError> {
    let mut stream = TcpStream::connect((host, port))
        .await
        .map_err(ProbeError::Unreachable)?;
    stream
        .write_all(&connection_request())
        .await
        .map_err(ProbeError::Unreachable)?;
    let mut header = [0u8; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|_| ProbeError::Protocol("no connection confirm"))?;
    if header[0] != 3 {
        return Err(ProbeError::Protocol("no TPKT header"));
    }
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    if !(11..=512).contains(&length) {
        return Err(ProbeError::Protocol("implausible TPKT length"));
    }
    let mut confirm = vec![0u8; length - 4];
    stream
        .read_exact(&mut confirm)
        .await
        .map_err(|_| ProbeError::Protocol("short connection confirm"))?;
    selected_protocol(&confirm)?;

    let connector = TlsConnector::from(Arc::new(client_config()));
    let name = ServerName::try_from(host.to_owned())
        .unwrap_or_else(|_| ServerName::try_from("rdp.invalid").expect("a valid DNS name"));
    let tls = connector
        .connect(name, stream)
        .await
        .map_err(ProbeError::Tls)?;
    let certificate = tls
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|chain| chain.first())
        .ok_or(ProbeError::Protocol("no certificate"))?;
    Ok(fingerprint(certificate.as_ref()))
}

/// TPKT + X.224 connection request with an RDP negotiation request.
fn connection_request() -> [u8; 19] {
    let mut request = [0u8; 19];
    // TPKT: version 3, length 19.
    request[..4].copy_from_slice(&[3, 0, 0, 19]);
    // X.224: length indicator 14, CR with credit 0, references and class 0.
    request[4..11].copy_from_slice(&[14, 0xe0, 0, 0, 0, 0, 0]);
    // RDP_NEG_REQ: type 1, flags 0, length 8, requested protocols.
    request[11..15].copy_from_slice(&[1, 0, 8, 0]);
    request[15..19].copy_from_slice(&REQUESTED_PROTOCOLS.to_le_bytes());
    request
}

/// The protocol the server chose from the X.224 connection confirm (after
/// the TPKT header); anything but TLS-based security ends the probe.
fn selected_protocol(confirm: &[u8]) -> Result<u32, ProbeError> {
    // Length indicator, CC (0xd0), references and class: 7 bytes.
    if confirm.len() < 7 || confirm[1] & 0xf0 != 0xd0 {
        return Err(ProbeError::Protocol("no connection confirm"));
    }
    let Some(negotiation) = confirm.get(7..15) else {
        // No negotiation response: an old server with standard RDP security.
        return Err(ProbeError::NoTls);
    };
    let value = u32::from_le_bytes([
        negotiation[4],
        negotiation[5],
        negotiation[6],
        negotiation[7],
    ]);
    match negotiation[0] {
        // RDP_NEG_RSP
        2 if value & REQUESTED_PROTOCOLS != 0 => Ok(value),
        2 => Err(ProbeError::NoTls),
        // RDP_NEG_FAILURE, e.g. SSL_NOT_ALLOWED_BY_SERVER
        3 => Err(ProbeError::NoTls),
        _ => Err(ProbeError::Protocol("unknown negotiation response")),
    }
}

/// SHA-256 over the DER certificate, upper-case hex pairs joined by `:`.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
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
/// from a CA (RDP servers mostly use self-signed ones) — but still checks
/// that the server holds its key.
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
    fn the_request_asks_for_tls_and_nla() {
        let request = connection_request();
        assert_eq!(usize::from(request[3]), request.len());
        assert_eq!(usize::from(request[4]), request.len() - 5);
        assert_eq!(&request[15..], &[0x0b, 0, 0, 0]);
    }

    #[test]
    fn the_confirm_says_whether_tls_follows() {
        let confirm = |kind: u8, value: u32| {
            let mut bytes = vec![14, 0xd0, 0, 0, 0, 0, 0, kind, 0, 8, 0];
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes
        };
        assert_eq!(selected_protocol(&confirm(2, 1)).unwrap(), 1);
        assert_eq!(selected_protocol(&confirm(2, 2)).unwrap(), 2);
        assert!(matches!(
            selected_protocol(&confirm(2, 0)),
            Err(ProbeError::NoTls)
        ));
        assert!(matches!(
            selected_protocol(&confirm(3, 2)),
            Err(ProbeError::NoTls)
        ));
        assert!(matches!(
            selected_protocol(&[6, 0xd0, 0, 0, 0, 0, 0]),
            Err(ProbeError::NoTls)
        ));
        assert!(matches!(
            selected_protocol(&[6, 0xe0, 0, 0, 0, 0, 0]),
            Err(ProbeError::Protocol(_))
        ));
    }

    #[test]
    fn fingerprints_look_like_freerdps() {
        assert_eq!(
            fingerprint(b"abc"),
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
    }
}
