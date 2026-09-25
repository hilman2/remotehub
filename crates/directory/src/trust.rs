//! Which certificates a domain controller may present (#144).
//!
//! The configured PEM may hold two kinds of certificates. A CA certificate
//! is a root: every certificate it signs for the right name counts. A
//! server certificate counts exactly as it is, whatever its name: many
//! domain controllers present only their own certificate, without the CA,
//! and an administrator can then trust that one after comparing its
//! fingerprint. It stops counting when the server gets a new one. Without
//! a PEM, the system's roots decide.

use std::sync::{Arc, Mutex};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};

/// Checks a server's certificate as described above, and keeps the chain
/// it presented, for [`crate::check`].
#[derive(Debug)]
pub(crate) struct Verifier {
    /// Verifies against the roots; none if there are no roots at all.
    roots: Option<Arc<WebPkiServerVerifier>>,
    servers: Vec<CertificateDer<'static>>,
    provider: Arc<CryptoProvider>,
    presented: Mutex<Vec<CertificateDer<'static>>>,
}

impl Verifier {
    /// Trusts the certificates in `ca_pem`, or the system's roots without it.
    pub(crate) fn new(ca_pem: Option<&str>) -> Result<Arc<Self>, String> {
        let describe = |e: &dyn std::fmt::Display| format!("CA certificate: {e}");
        let mut roots = RootCertStore::empty();
        let mut servers = Vec::new();
        match ca_pem {
            Some(pem) => {
                for certificate in CertificateDer::pem_slice_iter(pem.as_bytes()) {
                    let certificate = certificate.map_err(|e| describe(&e))?;
                    let (_, parsed) = x509_parser::parse_x509_certificate(&certificate)
                        .map_err(|e| describe(&e))?;
                    if parsed.is_ca() {
                        roots.add(certificate).map_err(|e| describe(&e))?;
                    } else {
                        servers.push(certificate);
                    }
                }
                if roots.is_empty() && servers.is_empty() {
                    return Err(describe(&"no certificates found"));
                }
            }
            None => {
                roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
            }
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let roots = if roots.is_empty() {
            None
        } else {
            Some(
                WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                    .build()
                    .map_err(|e| describe(&e))?,
            )
        };
        Ok(Arc::new(Verifier {
            roots,
            servers,
            provider,
            presented: Mutex::default(),
        }))
    }

    /// The chain the server presented last, its own certificate first.
    pub(crate) fn presented(&self) -> Vec<CertificateDer<'static>> {
        self.presented.lock().expect("presented lock").clone()
    }
}

/// A TLS client that trusts what `verifier` trusts.
pub(crate) fn client_config(verifier: Arc<Verifier>) -> Result<Arc<ClientConfig>, String> {
    let config = ClientConfig::builder_with_provider(verifier.provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.presented.lock().expect("presented lock") = std::iter::once(end_entity)
            .chain(intermediates)
            .map(|c| c.clone().into_owned())
            .collect();
        if self.servers.iter().any(|trusted| trusted == end_entity) {
            return Ok(ServerCertVerified::assertion());
        }
        match &self.roots {
            Some(roots) => {
                roots.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            }
            None => Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_pem_without_certificates() {
        assert!(
            Verifier::new(Some(""))
                .unwrap_err()
                .contains("no certificates")
        );
        let broken = "-----BEGIN CERTIFICATE-----\nnot base64\n-----END CERTIFICATE-----\n";
        assert!(Verifier::new(Some(broken)).is_err());
    }
}
