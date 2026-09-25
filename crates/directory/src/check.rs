//! A directory connection, checked step by step before it is saved (#144).
//! A failure names its step and a reason the UI can explain; the detail is
//! the server's or the library's own words.
//!
//! The first three steps (name, connection, TLS) run here on a plain socket,
//! because ldap3 reports them all alike and hides the certificate that made
//! TLS fail. An unknown CA comes back with the topmost certificate the server
//! presented, so an administrator can choose to trust it (see
//! [`crate::trust`]). Bind and search then go through [`LdapDirectory`] as
//! every sign-in does.

use std::time::Duration;

use rustls::CertificateError;
use rustls::pki_types::{CertificateDer, ServerName};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::ldap::{LdapConfig, LdapDirectory};
use crate::trust::{self, Verifier};

/// Where a check failed, in the order the steps run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Resolve,
    Connect,
    Tls,
    Bind,
    Search,
}

/// Why a step failed, as far as it is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    NotFound,
    Refused,
    Timeout,
    StartTlsRefused,
    UnknownCa,
    NameMismatch,
    Expired,
    InvalidCredentials,
    NoSuchBase,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub step: Step,
    pub reason: Reason,
    pub detail: String,
    /// With [`Reason::UnknownCa`]: the certificate to trust.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca: Option<PresentedCa>,
}

impl Failure {
    pub(crate) fn new(step: Step, reason: Reason, detail: impl ToString) -> Self {
        Failure {
            step,
            reason,
            detail: detail.to_string(),
            ca: None,
        }
    }
}

/// The topmost certificate a server presented: a CA's if it sends one,
/// otherwise its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresentedCa {
    /// Whether it is a CA's: trusted, it covers every certificate the CA
    /// signs; a server's own counts only until the server gets a new one.
    pub authority: bool,
    pub subject: String,
    /// SHA-256 of the certificate, in hex pairs, to compare with the CA.
    pub fingerprint: String,
    pub pem: String,
}

/// What a check that passed found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    /// Users the filter lets sign in, counted up to [`COUNT`].
    pub users: usize,
    /// There are more than [`COUNT`].
    pub more: bool,
    /// A few of their names, to recognise the directory by.
    pub sample: Vec<String>,
}

/// How many users a check counts.
const COUNT: i32 = 1000;
const SAMPLE: usize = 3;

/// Runs every step with `config` and stops at the first that fails.
pub async fn check(config: LdapConfig) -> Result<Found, Failure> {
    let target = Target::parse(&config.url, config.starttls)?;
    let limit = config.timeout;

    let addresses = match timeout(
        limit,
        tokio::net::lookup_host((target.host.as_str(), target.port)),
    )
    .await
    {
        Err(_) => return Err(Failure::new(Step::Resolve, Reason::Timeout, &target.host)),
        Ok(Err(error)) => return Err(Failure::new(Step::Resolve, Reason::NotFound, error)),
        Ok(Ok(addresses)) => addresses.collect::<Vec<_>>(),
    };
    let address = *addresses
        .first()
        .ok_or_else(|| Failure::new(Step::Resolve, Reason::NotFound, &target.host))?;

    let mut stream = match timeout(limit, TcpStream::connect(address)).await {
        Err(_) => return Err(Failure::new(Step::Connect, Reason::Timeout, address)),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            return Err(Failure::new(Step::Connect, Reason::Refused, address));
        }
        Ok(Err(error)) => return Err(Failure::new(Step::Connect, Reason::Other, error)),
        Ok(Ok(stream)) => stream,
    };
    if config.starttls {
        start_tls(&mut stream, limit).await?;
    }
    handshake(stream, &target.host, config.ca_pem.as_deref(), limit).await?;

    let directory =
        LdapDirectory::new(config).map_err(|e| Failure::new(Step::Tls, Reason::Other, e))?;
    let (names, more) = directory.users(COUNT).await?;
    Ok(Found {
        users: names.len(),
        more,
        sample: names.into_iter().take(SAMPLE).collect(),
    })
}

/// Host and port of an `ldaps://` or `ldap://` URL.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    host: String,
    port: u16,
}

impl Target {
    fn parse(url: &str, starttls: bool) -> Result<Self, Failure> {
        let invalid = || {
            Failure::new(
                Step::Resolve,
                Reason::Other,
                format!("not an LDAP URL: {url}"),
            )
        };
        let lower = url.trim().to_ascii_lowercase();
        let (rest, default_port) = if let Some(rest) = lower.strip_prefix("ldaps://") {
            (rest, 636)
        } else if let Some(rest) = lower.strip_prefix("ldap://") {
            if !starttls {
                return Err(Failure::new(
                    Step::Tls,
                    Reason::Other,
                    "ldap:// needs StartTLS",
                ));
            }
            (rest, 389)
        } else {
            return Err(invalid());
        };
        let authority = rest.split('/').next().unwrap_or_default();
        let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
            // An IPv6 address: [fd00::5]:636
            let (host, after) = bracketed.split_once(']').ok_or_else(invalid)?;
            (host, after.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            }
        };
        if host.is_empty() {
            return Err(invalid());
        }
        let port = match port {
            Some(port) => port.parse().map_err(|_| invalid())?,
            None => default_port,
        };
        Ok(Target {
            host: host.to_owned(),
            port,
        })
    }
}

const START_TLS_OID: &[u8] = b"1.3.6.1.4.1.1466.20037";

/// The StartTLS extended operation (RFC 4511, 4.14) as message 1, in BER:
/// SEQUENCE { messageID 1, [APPLICATION 23] { [0] the OID } }.
fn start_tls_request() -> Vec<u8> {
    let oid = START_TLS_OID.len() as u8;
    let mut request = vec![
        0x30,
        3 + 2 + 2 + oid,
        0x02,
        0x01,
        0x01,
        0x77,
        2 + oid,
        0x80,
        oid,
    ];
    request.extend_from_slice(START_TLS_OID);
    request
}

/// Asks the server to start TLS on `stream`; its answer must be success.
async fn start_tls(stream: &mut TcpStream, limit: Duration) -> Result<(), Failure> {
    let failed = |e: &dyn std::fmt::Display| Failure::new(Step::Tls, Reason::StartTlsRefused, e);
    let exchange = async {
        stream.write_all(&start_tls_request()).await?;
        // The answer is short; its result code follows the tag of the
        // ExtendedResponse (0x78) and its length, as ENUMERATED 0x0a 0x01.
        let mut answer = [0u8; 256];
        let read = stream.read(&mut answer).await?;
        Ok::<_, std::io::Error>(answer[..read].to_vec())
    };
    let answer = timeout(limit, exchange)
        .await
        .map_err(|_| Failure::new(Step::Tls, Reason::Timeout, "StartTLS"))?
        .map_err(|e| failed(&e))?;
    match result_code(&answer) {
        Some(0) => Ok(()),
        Some(code) => Err(failed(&format!("result code {code}"))),
        None => Err(failed(&"no StartTLS answer")),
    }
}

/// The result code of an LDAP ExtendedResponse.
fn result_code(answer: &[u8]) -> Option<u8> {
    let response = answer.iter().position(|&b| b == 0x78)?;
    answer[response..]
        .windows(3)
        .find(|w| w[0] == 0x0a && w[1] == 0x01)
        .map(|w| w[2])
}

/// A TLS handshake that trusts what a sign-in would trust: `ca_pem`, or the
/// system's roots.
async fn handshake(
    stream: TcpStream,
    host: &str,
    ca_pem: Option<&str>,
    limit: Duration,
) -> Result<(), Failure> {
    let other = |e: &dyn std::fmt::Display| Failure::new(Step::Tls, Reason::Other, e);
    let verifier = Verifier::new(ca_pem).map_err(|e| other(&e))?;
    let config = trust::client_config(verifier.clone()).map_err(|e| other(&e))?;
    let name = ServerName::try_from(host.to_owned()).map_err(|e| other(&e))?;
    let connecting = TlsConnector::from(config).connect(name, stream);
    match timeout(limit, connecting).await {
        Err(_) => Err(Failure::new(Step::Tls, Reason::Timeout, host)),
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => {
            let rustls_error = error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<rustls::Error>());
            let reason = match rustls_error {
                Some(rustls::Error::InvalidCertificate(certificate)) => match certificate {
                    CertificateError::UnknownIssuer => Reason::UnknownCa,
                    CertificateError::NotValidForName
                    | CertificateError::NotValidForNameContext { .. } => Reason::NameMismatch,
                    CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                        Reason::Expired
                    }
                    _ => Reason::Other,
                },
                _ => Reason::Other,
            };
            let mut failure = Failure::new(Step::Tls, reason, &error);
            if reason == Reason::UnknownCa {
                failure.ca = verifier.presented().last().and_then(presented);
            }
            Err(failure)
        }
    }
}

fn presented(certificate: &CertificateDer<'_>) -> Option<PresentedCa> {
    let (_, parsed) = x509_parser::parse_x509_certificate(certificate).ok()?;
    let fingerprint = Sha256::digest(certificate)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":");
    Some(PresentedCa {
        authority: parsed.is_ca(),
        subject: parsed.subject().to_string(),
        fingerprint,
        pem: pem(certificate),
    })
}

/// A certificate as PEM, in lines of 64 characters.
fn pem(certificate: &[u8]) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(certificate);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in encoded.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_host_and_port_of_both_schemes() {
        let target = |url: &str, starttls: bool| Target::parse(url, starttls).map_err(|f| f.reason);
        assert_eq!(
            target("ldaps://DC.example.com", false),
            Ok(Target {
                host: "dc.example.com".into(),
                port: 636
            })
        );
        assert_eq!(
            target("ldap://dc.example.com:3268/", true),
            Ok(Target {
                host: "dc.example.com".into(),
                port: 3268
            })
        );
        assert_eq!(
            target("ldaps://[fd00::5]:1636", false),
            Ok(Target {
                host: "fd00::5".into(),
                port: 1636
            })
        );
        assert_eq!(target("ldap://dc.example.com", false), Err(Reason::Other));
        for wrong in ["https://dc", "ldaps://", "ldaps://dc:port"] {
            assert_eq!(target(wrong, false), Err(Reason::Other), "{wrong}");
        }
    }

    #[test]
    fn asks_for_start_tls_in_ber() {
        let request = start_tls_request();
        // The outer length covers the rest, the operation's its OID.
        assert_eq!(usize::from(request[1]), request.len() - 2);
        assert_eq!(usize::from(request[6]), request.len() - 7);
        assert_eq!(&request[..9], b"\x30\x1d\x02\x01\x01\x77\x18\x80\x16");
    }

    #[test]
    fn reads_the_result_code_of_start_tls() {
        // Success, as Samba answers, and "unavailable" (52).
        let answer = |code: u8| {
            vec![
                0x30, 0x0c, 0x02, 0x01, 0x01, 0x78, 0x07, 0x0a, 0x01, code, 0x04, 0x00, 0x04, 0x00,
            ]
        };
        assert_eq!(result_code(&answer(0)), Some(0));
        assert_eq!(result_code(&answer(52)), Some(52));
        assert_eq!(result_code(b"\x30\x03\x02\x01\x01"), None);
    }

    #[test]
    fn writes_pem_in_lines_of_64() {
        let written = pem(&[0u8; 100]);
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.first(), Some(&"-----BEGIN CERTIFICATE-----"));
        assert_eq!(lines.last(), Some(&"-----END CERTIFICATE-----"));
        assert_eq!(lines[1].len(), 64);
        // It reads back as PEM, and zeros are no certificate.
        assert!(
            Verifier::new(Some(&written))
                .unwrap_err()
                .contains("CA certificate")
        );
    }
}
