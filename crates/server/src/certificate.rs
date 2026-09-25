//! A TLS certificate of your own for Caddy of the ops package (#146): read
//! from PEM or PFX and checked against the host before Caddy gets it.
//! `caddy.rs` hands it over; this module only reads and checks.

use rustls::SignatureAlgorithm;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Serialize;
use sha2::{Digest, Sha256};
use x509_parser::extensions::GeneralName;
use zeroize::Zeroizing;

/// A certificate chain, the server's own first, and its key.
pub struct Uploaded {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

/// What a certificate says about itself, for the settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Info {
    pub subject: String,
    pub issuer: String,
    /// The DNS names it covers.
    pub names: Vec<String>,
    /// Seconds since 1970.
    pub not_before: i64,
    pub not_after: i64,
    /// SHA-256 of the certificate, in hex pairs.
    pub fingerprint: String,
}

/// Why a certificate is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    #[error("no certificate could be read")]
    Unreadable,
    #[error("no private key could be read")]
    NoKey,
    #[error("the PFX file does not open with this password")]
    PfxPassword,
    #[error("the key does not belong to the certificate")]
    KeyMismatch,
    #[error("the key is neither RSA of at least 2048 bits nor ECDSA P-256 or P-384")]
    UnsupportedKey,
    #[error("the certificate does not cover the host's name")]
    WrongName,
    #[error("the certificate is not valid yet")]
    NotYetValid,
    #[error("the certificate has run out, or runs out within a day")]
    Expired,
}

/// A certificate that passed [`check`], and what it says.
pub struct Checked {
    pub info: Info,
    /// The server's certificate comes with the one that signed it, or signed
    /// itself. Without it, some clients cannot build the chain.
    pub chain_complete: bool,
}

/// Reads PEM: the certificates, the server's own first, and the key
/// (PKCS#8, PKCS#1 or SEC1).
pub fn from_pem(certificates: &str, key: &str) -> Result<Uploaded, Refusal> {
    let chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(certificates.as_bytes())
            .collect::<Result<_, _>>()
            .map_err(|_| Refusal::Unreadable)?;
    if chain.is_empty() {
        return Err(Refusal::Unreadable);
    }
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).map_err(|_| Refusal::NoKey)?;
    Ok(Uploaded { chain, key })
}

/// Reads a PFX (PKCS#12) file, as Windows exports one: the key with its
/// chain.
pub fn from_pfx(data: &[u8], password: &str) -> Result<Uploaded, Refusal> {
    let store = p12_keystore::KeyStore::from_pkcs12(
        data,
        password,
        p12_keystore::Pkcs12ImportPolicy::Relaxed,
    )
    .map_err(|error| match error {
        // A wrong password shows as a MAC that does not verify.
        p12_keystore::error::Error::MacError(_) => Refusal::PfxPassword,
        _ => Refusal::Unreadable,
    })?;
    let (_, chain) = store.private_key_chain().ok_or(Refusal::NoKey)?;
    let certificates: Vec<CertificateDer<'static>> = chain
        .certs()
        .iter()
        .map(|c| CertificateDer::from(c.as_der().to_vec()))
        .collect();
    if certificates.is_empty() {
        return Err(Refusal::Unreadable);
    }
    Ok(Uploaded {
        chain: certificates,
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(chain.key().as_der().to_vec())),
    })
}

/// What `certificate` says about itself; none if it is no certificate.
pub fn info(certificate: &[u8]) -> Option<Info> {
    let (_, parsed) = x509_parser::parse_x509_certificate(certificate).ok()?;
    let names = parsed
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|name| match name {
                    GeneralName::DNSName(dns) => Some(dns.to_ascii_lowercase()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Info {
        subject: parsed.subject().to_string(),
        issuer: parsed.issuer().to_string(),
        names,
        not_before: parsed.validity().not_before.timestamp(),
        not_after: parsed.validity().not_after.timestamp(),
        fingerprint: fingerprint(certificate),
    })
}

pub fn fingerprint(certificate: &[u8]) -> String {
    Sha256::digest(certificate)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Whether `names` cover `host`: exactly, or through a wildcard for one label.
fn covers(names: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    names.iter().any(|name| {
        name == &host
            || name.strip_prefix("*.").is_some_and(|parent| {
                host.split_once('.')
                    .is_some_and(|(label, rest)| !label.is_empty() && rest == parent)
            })
    })
}

/// A day: a certificate that runs out sooner would be gone before anyone
/// notices.
const LEAST_LIFE: i64 = 24 * 3600;

/// Checks `uploaded` for `host` at `now` (seconds since 1970): the key
/// belongs to the certificate and is of a kind browsers take, the names
/// cover the host, and it is valid now and for at least a day more.
pub fn check(uploaded: &Uploaded, host: &str, now: i64) -> Result<Checked, Refusal> {
    let leaf = &uploaded.chain[0];
    let (_, parsed) = x509_parser::parse_x509_certificate(leaf).map_err(|_| Refusal::Unreadable)?;
    let signer = rustls::crypto::ring::sign::any_supported_type(&uploaded.key)
        .map_err(|_| Refusal::UnsupportedKey)?;
    if signer.algorithm() == SignatureAlgorithm::ED25519 {
        return Err(Refusal::UnsupportedKey);
    }
    let public = signer.public_key().ok_or(Refusal::UnsupportedKey)?;
    if public.as_ref() != parsed.tbs_certificate.subject_pki.raw {
        return Err(Refusal::KeyMismatch);
    }
    let info = info(leaf).ok_or(Refusal::Unreadable)?;
    if !covers(&info.names, host) {
        return Err(Refusal::WrongName);
    }
    if info.not_before > now {
        return Err(Refusal::NotYetValid);
    }
    if info.not_after < now + LEAST_LIFE {
        return Err(Refusal::Expired);
    }
    let chain_complete = uploaded.chain.len() > 1 || parsed.subject() == parsed.issuer();
    Ok(Checked {
        info,
        chain_complete,
    })
}

const KEY_FIELD: &str = "tls_key";

/// The stored certificate of your own, with its key; none without one.
pub async fn stored(
    db: &sqlx::PgPool,
    vault: &remotehub_vault::DynVault,
) -> Result<Option<Uploaded>, crate::secrets::SecretError> {
    let Some((id, chain)) =
        sqlx::query_as::<_, (uuid::Uuid, String)>("SELECT id, chain_pem FROM certificate")
            .fetch_optional(db)
            .await?
    else {
        return Ok(None);
    };
    let Some(key) = crate::secrets::load(db, vault, id, 1, KEY_FIELD).await? else {
        return Ok(None);
    };
    let key = std::str::from_utf8(&key).unwrap_or_default();
    Ok(from_pem(&chain, key).ok())
}

/// Stores `uploaded` in place of the certificate before, its key sealed.
pub async fn store(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    vault: &remotehub_vault::DynVault,
    uploaded: &Uploaded,
    not_after: i64,
    by: uuid::Uuid,
) -> Result<(), crate::secrets::SecretError> {
    remove(tx).await?;
    let (chain, key) = to_pem(uploaded);
    let id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO certificate (chain_pem, not_after, updated_by)
         VALUES ($1, to_timestamp($2), $3) RETURNING id",
    )
    .bind(&chain)
    .bind(not_after as f64)
    .bind(by)
    .fetch_one(&mut **tx)
    .await?;
    crate::secrets::store(&mut **tx, vault, id, 1, KEY_FIELD, key.as_bytes()).await
}

/// Removes the stored certificate and its key; whether there was one.
pub async fn remove(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<bool, sqlx::Error> {
    let removed: Option<uuid::Uuid> = sqlx::query_scalar("DELETE FROM certificate RETURNING id")
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(id) = removed {
        sqlx::query("DELETE FROM secret_fields WHERE owner_id = $1")
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(removed.is_some())
}

/// The chain and the key as PEM, for Caddy's files.
pub fn to_pem(uploaded: &Uploaded) -> (String, Zeroizing<String>) {
    let block = |label: &str, der: &[u8]| {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(der);
        let mut pem = format!("-----BEGIN {label}-----\n");
        for line in encoded.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
            pem.push('\n');
        }
        pem.push_str(&format!("-----END {label}-----\n"));
        pem
    };
    let chain = uploaded
        .chain
        .iter()
        .map(|c| block("CERTIFICATE", c))
        .collect();
    let key = match &uploaded.key {
        PrivateKeyDer::Pkcs1(key) => block("RSA PRIVATE KEY", key.secret_pkcs1_der()),
        PrivateKeyDer::Sec1(key) => block("EC PRIVATE KEY", key.secret_sec1_der()),
        PrivateKeyDer::Pkcs8(key) => block("PRIVATE KEY", key.secret_pkcs8_der()),
        _ => String::new(),
    };
    (chain, Zeroizing::new(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600;

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// A self-signed certificate for `names`, valid from `from` to `until`
    /// days from now, and its key as PEM.
    fn made(names: &[&str], from: i64, until: i64) -> (String, String) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params =
            rcgen::CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .unwrap();
        params.not_before = time::OffsetDateTime::from_unix_timestamp(now() + from * DAY).unwrap();
        params.not_after = time::OffsetDateTime::from_unix_timestamp(now() + until * DAY).unwrap();
        let certificate = params.self_signed(&key).unwrap();
        (certificate.pem(), key.serialize_pem())
    }

    fn checked(names: &[&str], from: i64, until: i64, host: &str) -> Result<Checked, Refusal> {
        let (certificate, key) = made(names, from, until);
        check(&from_pem(&certificate, &key).unwrap(), host, now())
    }

    #[test]
    fn a_fitting_certificate_passes_and_says_what_it_is() {
        let passed = checked(&["remotehub.example.com"], -1, 90, "RemoteHub.example.com").unwrap();
        assert_eq!(passed.info.names, ["remotehub.example.com"]);
        assert_eq!(passed.info.fingerprint.len(), 32 * 3 - 1);
        assert!(passed.chain_complete, "self-signed needs no more");
        assert!(checked(&["*.example.com"], -1, 90, "remotehub.example.com").is_ok());
    }

    #[test]
    fn names_time_and_key_refuse_a_certificate() {
        assert_eq!(
            checked(&["other.example.com"], -1, 90, "remotehub.example.com").err(),
            Some(Refusal::WrongName)
        );
        // A wildcard covers one label, not two, and not the parent itself.
        for host in ["a.remotehub.example.com", "example.com"] {
            assert_eq!(
                checked(&["*.example.com"], -1, 90, host).err(),
                Some(Refusal::WrongName),
                "{host}"
            );
        }
        assert_eq!(
            checked(&["remotehub.example.com"], 1, 90, "remotehub.example.com").err(),
            Some(Refusal::NotYetValid)
        );
        assert_eq!(
            checked(&["remotehub.example.com"], -90, 0, "remotehub.example.com").err(),
            Some(Refusal::Expired)
        );

        let (certificate, _) = made(&["remotehub.example.com"], -1, 90);
        let (_, other_key) = made(&["remotehub.example.com"], -1, 90);
        let mismatched = from_pem(&certificate, &other_key).unwrap();
        assert_eq!(
            check(&mismatched, "remotehub.example.com", now()).err(),
            Some(Refusal::KeyMismatch)
        );
        assert_eq!(
            from_pem("nothing", &other_key).err(),
            Some(Refusal::Unreadable)
        );
        assert_eq!(
            from_pem(&certificate, "nothing").err(),
            Some(Refusal::NoKey)
        );
    }

    #[test]
    fn a_pfx_opens_with_its_password_only() {
        let (certificate, key) = made(&["remotehub.example.com"], -1, 90);
        let pem = from_pem(&certificate, &key).unwrap();
        let PrivateKeyDer::Pkcs8(pkcs8) = &pem.key else {
            panic!("rcgen writes PKCS#8");
        };
        let chain = p12_keystore::PrivateKeyChain::new(
            [1u8; 20].to_vec(),
            p12_keystore::PrivateKey::from_der(pkcs8.secret_pkcs8_der()).unwrap(),
            [p12_keystore::Certificate::from_der(&pem.chain[0]).unwrap()],
        );
        let mut store = p12_keystore::KeyStore::new();
        store.add_entry(
            "remotehub",
            p12_keystore::KeyStoreEntry::PrivateKeyChain(chain),
        );
        let pfx = store.writer("secret").write().unwrap();

        assert_eq!(from_pfx(&pfx, "wrong").err(), Some(Refusal::PfxPassword));
        let opened = from_pfx(&pfx, "secret").unwrap();
        assert!(check(&opened, "remotehub.example.com", now()).is_ok());
        let (chain, key) = to_pem(&opened);
        assert!(
            from_pem(&chain, &key).is_ok(),
            "the files Caddy gets read back"
        );
    }
}
