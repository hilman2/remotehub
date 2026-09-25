//! Master keys kept for the organisation recovery key (#96, ADR 0009).
//!
//! A database backup and the organisation's private key then restore the
//! master key file: one secret to keep safe instead of two. Each master key
//! is sealed for the recovery key's public key: ECDH on P-256 with a key pair
//! made for this seal alone, HKDF-SHA-256 into an XChaCha20-Poly1305 key, and
//! the master key's id and version as associated data, so a sealed key
//! cannot pass for another version.

use data_encoding::BASE32_NOPAD;
use hkdf::Hkdf;
use p256::ecdh::diffie_hellman;
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::{KEY_LEN, Key, VaultError, decrypt, encrypt};

/// Length of an uncompressed P-256 point.
const POINT_LEN: usize = 65;
/// Length of a P-256 private key (its scalar).
const SCALAR_LEN: usize = 32;

/// A master key sealed for a recovery key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escrowed {
    /// The public half of the key pair made for this seal, uncompressed.
    pub ephemeral: Vec<u8>,
    /// The master key: nonce ‖ ciphertext.
    pub sealed: Vec<u8>,
}

fn aad(kek_id: &str, version: i32) -> Vec<u8> {
    format!("remotehub|escrow|{kek_id}|{version}").into_bytes()
}

fn sealing_key(private: &SecretKey, public: &PublicKey) -> Key {
    let shared = diffie_hellman(private.to_nonzero_scalar(), public.as_affine());
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    Hkdf::<Sha256>::new(Some(&[0u8; 32]), shared.raw_secret_bytes())
        .expand(b"remotehub master key escrow", key.as_mut_slice())
        .expect("32 bytes are a valid HKDF output");
    key
}

fn point(bytes: &[u8]) -> Result<PublicKey, VaultError> {
    if bytes.len() != POINT_LEN {
        return Err(VaultError::InvalidPublicKey);
    }
    PublicKey::from_sec1_bytes(bytes).map_err(|_| VaultError::InvalidPublicKey)
}

/// Whether `bytes` are an uncompressed point on P-256, as browsers export
/// a public key (`raw`).
pub fn is_public_key(bytes: &[u8]) -> bool {
    point(bytes).is_ok()
}

fn uncompressed(key: &PublicKey) -> Vec<u8> {
    key.to_sec1_point(false).as_bytes().to_vec()
}

/// The public key belonging to a private key's scalar.
pub fn public_key_of(private_key: &[u8]) -> Result<Vec<u8>, VaultError> {
    let secret = SecretKey::from_slice(private_key).map_err(|_| VaultError::Open)?;
    Ok(uncompressed(&secret.public_key()))
}

/// Seals master key `id`/`version` for the recovery key `public_key`.
pub fn seal_for(
    public_key: &[u8],
    kek_id: &str,
    version: i32,
    master: &Key,
) -> Result<Escrowed, VaultError> {
    let recipient = point(public_key)?;
    let ephemeral = loop {
        // Nearly every 32 random bytes are a scalar; the rest are below the
        // order by so little that a retry never happens in practice.
        let bytes = crate::random_key();
        if let Ok(secret) = SecretKey::from_slice(bytes.as_slice()) {
            break secret;
        }
    };
    Ok(Escrowed {
        ephemeral: uncompressed(&ephemeral.public_key()),
        sealed: encrypt(
            &sealing_key(&ephemeral, &recipient),
            &aad(kek_id, version),
            master.as_slice(),
        ),
    })
}

/// The master key; fails if `private_key` is not the one it was sealed for,
/// or `kek_id`/`version` are not the ones it was sealed as.
pub fn open_with(
    private_key: &[u8],
    escrowed: &Escrowed,
    kek_id: &str,
    version: i32,
) -> Result<Key, VaultError> {
    let secret = SecretKey::from_slice(private_key).map_err(|_| VaultError::Open)?;
    let key = sealing_key(&secret, &point(&escrowed.ephemeral)?);
    let plain = decrypt(&key, &aad(kek_id, version), &escrowed.sealed)?;
    let master: [u8; KEY_LEN] = plain.as_slice().try_into().map_err(|_| VaultError::Open)?;
    Ok(Zeroizing::new(master))
}

/// The private key's scalar as the browser prints it for the safe (base32
/// in groups of four, `web/src/lib/vault/crypto.ts`), typed in any case,
/// with or without dashes and spaces. `None` if it is not one.
pub fn parse_printed(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    let clean = Zeroizing::new(
        text.chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>(),
    );
    let bytes = Zeroizing::new(BASE32_NOPAD.decode(clean.as_bytes()).ok()?);
    (bytes.len() == SCALAR_LEN).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Zeroizing<Vec<u8>>, Vec<u8>) {
        let private = Zeroizing::new(crate::random_key().to_vec());
        let public = public_key_of(&private).unwrap();
        (private, public)
    }

    #[test]
    fn opens_only_with_the_private_key_and_for_the_same_version() {
        let (private, public) = pair();
        let master = crate::random_key();
        let escrowed = seal_for(&public, "file", 2, &master).unwrap();
        assert!(!escrowed.sealed.windows(8).any(|w| w == &master[..8]));
        assert_eq!(open_with(&private, &escrowed, "file", 2).unwrap(), master);
        assert_eq!(
            open_with(&private, &escrowed, "file", 1),
            Err(VaultError::Open)
        );
        let (other, _) = pair();
        assert_eq!(
            open_with(&other, &escrowed, "file", 2),
            Err(VaultError::Open)
        );
        // Every seal has a key pair of its own.
        assert_ne!(
            seal_for(&public, "file", 2, &master).unwrap().ephemeral,
            escrowed.ephemeral
        );
    }

    #[test]
    fn takes_only_points_on_the_curve() {
        let (_, public) = pair();
        assert!(is_public_key(&public));
        let mut bent = public.clone();
        bent[64] ^= 1;
        assert!(!is_public_key(&bent));
        assert!(!is_public_key(&public[..33]));
        assert_eq!(
            seal_for(&bent, "file", 1, &crate::random_key()),
            Err(VaultError::InvalidPublicKey)
        );
    }

    #[test]
    fn reads_the_private_key_as_the_browser_prints_it() {
        // Printed by `privateKeyText` in crypto.ts for the bytes 1 to 32.
        let printed = "AEBA-GBAF-AYDQ-QCIK-BMGA-2DQP-CAIR-EEYU-CULB-OGAZ-DINR-YHI6-D4QA";
        let bytes: Vec<u8> = (1..=32).collect();
        assert_eq!(parse_printed(printed).unwrap().as_slice(), bytes);
        assert_eq!(
            parse_printed(&printed.to_lowercase().replace('-', " "))
                .unwrap()
                .as_slice(),
            bytes
        );
        assert!(parse_printed(&printed[..printed.len() - 2]).is_none());
        assert!(parse_printed("not a key").is_none());
    }

    #[test]
    fn finds_the_public_key_the_browser_made_with_the_private_key() {
        // A key pair from `newOrganisationKey` in crypto.ts (WebCrypto).
        let printed = "G2WH-JW4A-XIDO-EZUI-Q7E3-HIIK-ZMD3-C4PT-SFHG-SPJ2-W5I6-WOUB-PH7A";
        let public = "BNmN5cJFR128t90I/Vw1nHpbkj3B0cwp98tyffyM5/xdv9ZzVvsooPT7MS0BbDCqRh1+xYfZz1otG1hF5xVaAQ8=";
        let private = parse_printed(printed).unwrap();
        use base64::Engine;
        let expected = base64::engine::general_purpose::STANDARD
            .decode(public)
            .unwrap();
        assert_eq!(public_key_of(&private).unwrap(), expected);
    }
}
