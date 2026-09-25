//! Security keys and passkeys as the second factor of directory users
//! (#129): the checks of a WebAuthn registration and assertion, as far as a
//! second factor needs them.
//!
//! Only ES256 (ECDSA on P-256 with SHA-256): every current passkey and
//! security key offers it, and `p256` is in the tree already, where
//! `webauthn-rs` would need OpenSSL. Attestation is not checked: remotehub
//! trusts no particular maker, so the key's public half comes from the
//! browser (`getPublicKey()`) and is bound to the credential by the checks
//! of the registration.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::pkcs8::DecodePublicKey;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// COSE's number for ES256, as `getPublicKeyAlgorithm()` reports it.
pub const ES256: i64 = -7;

/// Who the keys belong to: the host people open, and its origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelyingParty {
    /// The host name alone, e.g. `remotehub.example.com`.
    pub id: String,
    /// e.g. `https://remotehub.example.com`
    pub origin: String,
}

impl RelyingParty {
    /// From remotehub's public origin (`REMOTEHUB_PUBLIC_URL`).
    pub fn of(origin: &str) -> Self {
        let host = origin
            .split_once("://")
            .map_or(origin, |(_, rest)| rest)
            .split(['/', ':'])
            .next()
            .unwrap_or_default();
        RelyingParty {
            id: host.to_owned(),
            origin: origin.trim_end_matches('/').to_owned(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebauthnError {
    #[error("the answer is not well-formed: {0}")]
    Malformed(&'static str),
    #[error("the answer is for another ceremony, challenge or origin")]
    Mismatch,
    #[error("the key did not confirm that someone was present")]
    NotPresent,
    #[error("only ES256 keys are taken")]
    Algorithm,
    #[error("the signature does not verify")]
    Signature,
    /// The key's counter went backwards: a copy of the key may be in use.
    #[error("the signature counter did not grow")]
    Counter,
}

fn decode(text: &str, what: &'static str) -> Result<Vec<u8>, WebauthnError> {
    URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .map_err(|_| WebauthnError::Malformed(what))
}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
}

/// `clientDataJSON` must name the ceremony, the challenge and the origin.
fn check_client_data(
    rp: &RelyingParty,
    kind: &str,
    challenge: &[u8],
    client_data: &[u8],
) -> Result<(), WebauthnError> {
    let data: ClientData = serde_json::from_slice(client_data)
        .map_err(|_| WebauthnError::Malformed("clientDataJSON"))?;
    let sent = decode(&data.challenge, "challenge")?;
    if data.kind != kind || sent != challenge || data.origin != rp.origin {
        return Err(WebauthnError::Mismatch);
    }
    Ok(())
}

/// The fixed start of `authenticatorData`: the relying party's hash, the
/// flags and the counter. Returns the counter.
fn check_authenticator_data(rp: &RelyingParty, data: &[u8]) -> Result<u32, WebauthnError> {
    if data.len() < 37 {
        return Err(WebauthnError::Malformed("authenticatorData"));
    }
    if data[..32] != Sha256::digest(rp.id.as_bytes())[..] {
        return Err(WebauthnError::Mismatch);
    }
    // Bit 0: user present.
    if data[32] & 0x01 == 0 {
        return Err(WebauthnError::NotPresent);
    }
    Ok(u32::from_be_bytes([data[33], data[34], data[35], data[36]]))
}

/// What the browser sends when a key is registered, base64url throughout.
#[derive(Debug, Deserialize)]
pub struct Registration {
    #[serde(rename = "rawId")]
    pub raw_id: String,
    pub response: RegistrationResponse,
}

#[derive(Debug, Deserialize)]
pub struct RegistrationResponse {
    #[serde(rename = "clientDataJSON")]
    pub client_data: String,
    #[serde(rename = "authenticatorData")]
    pub authenticator_data: String,
    /// SubjectPublicKeyInfo (DER), from `getPublicKey()`.
    #[serde(rename = "publicKey")]
    pub public_key: String,
    #[serde(rename = "publicKeyAlgorithm")]
    pub algorithm: i64,
}

/// A registered key: its credential ID, public key (SEC1, uncompressed)
/// and counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewKey {
    pub credential_id: Vec<u8>,
    pub public_key: Vec<u8>,
    pub counter: u32,
}

/// Checks a registration against the challenge it answers.
pub fn register(
    rp: &RelyingParty,
    challenge: &[u8],
    answer: &Registration,
) -> Result<NewKey, WebauthnError> {
    if answer.response.algorithm != ES256 {
        return Err(WebauthnError::Algorithm);
    }
    let response = &answer.response;
    check_client_data(
        rp,
        "webauthn.create",
        challenge,
        &decode(&response.client_data, "clientDataJSON")?,
    )?;
    let data = decode(&response.authenticator_data, "authenticatorData")?;
    let counter = check_authenticator_data(rp, &data)?;
    // Bit 6: attested credential data follows, with the credential's ID
    // after the authenticator's AAGUID.
    if data[32] & 0x40 == 0 || data.len() < 55 {
        return Err(WebauthnError::Malformed("authenticatorData"));
    }
    let length = usize::from(u16::from_be_bytes([data[53], data[54]]));
    let credential_id = data
        .get(55..55 + length)
        .ok_or(WebauthnError::Malformed("authenticatorData"))?;
    if credential_id != decode(&answer.raw_id, "rawId")? {
        return Err(WebauthnError::Mismatch);
    }
    let key = p256::PublicKey::from_public_key_der(&decode(&response.public_key, "publicKey")?)
        .map_err(|_| WebauthnError::Algorithm)?;
    Ok(NewKey {
        credential_id: credential_id.to_vec(),
        public_key: key.to_sec1_point(false).as_bytes().to_vec(),
        counter,
    })
}

/// What the browser sends when a key signs a challenge.
#[derive(Debug, Deserialize)]
pub struct Assertion {
    #[serde(rename = "rawId")]
    pub raw_id: String,
    pub response: AssertionResponse,
}

#[derive(Debug, Deserialize)]
pub struct AssertionResponse {
    #[serde(rename = "clientDataJSON")]
    pub client_data: String,
    #[serde(rename = "authenticatorData")]
    pub authenticator_data: String,
    pub signature: String,
}

impl Assertion {
    pub fn credential_id(&self) -> Result<Vec<u8>, WebauthnError> {
        decode(&self.raw_id, "rawId")
    }
}

/// Checks a signature of the stored key over the challenge. Returns the
/// key's new counter.
pub fn verify(
    rp: &RelyingParty,
    challenge: &[u8],
    public_key: &[u8],
    stored_counter: u32,
    answer: &Assertion,
) -> Result<u32, WebauthnError> {
    let response = &answer.response;
    let client_data = decode(&response.client_data, "clientDataJSON")?;
    check_client_data(rp, "webauthn.get", challenge, &client_data)?;
    let data = decode(&response.authenticator_data, "authenticatorData")?;
    let counter = check_authenticator_data(rp, &data)?;
    let key = VerifyingKey::from_sec1_bytes(public_key)
        .map_err(|_| WebauthnError::Malformed("stored key"))?;
    let signature = Signature::from_der(&decode(&response.signature, "signature")?)
        .map_err(|_| WebauthnError::Malformed("signature"))?;
    let signed = [data.as_slice(), &Sha256::digest(&client_data)].concat();
    key.verify(&signed, &signature)
        .map_err(|_| WebauthnError::Signature)?;
    // Keys without a counter always send 0; one that has one must count up.
    if (counter != 0 || stored_counter != 0) && counter <= stored_counter {
        return Err(WebauthnError::Counter);
    }
    Ok(counter)
}

#[cfg(test)]
mod tests {
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer;
    use p256::pkcs8::EncodePublicKey;
    use serde_json::json;

    use super::*;

    /// A software authenticator: what a security key computes, for tests.
    struct Authenticator {
        key: SigningKey,
        credential_id: Vec<u8>,
        counter: u32,
    }

    fn rp() -> RelyingParty {
        RelyingParty::of("https://remotehub.example.com")
    }

    fn b64(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn client_data(kind: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
        json!({ "type": kind, "challenge": b64(challenge), "origin": origin, "crossOrigin": false })
            .to_string()
            .into_bytes()
    }

    impl Authenticator {
        fn new() -> Self {
            Authenticator {
                key: SigningKey::from_slice(&[7u8; 32]).unwrap(),
                credential_id: vec![9u8; 16],
                counter: 0,
            }
        }

        fn data(&self, rp_id: &str, flags: u8) -> Vec<u8> {
            let mut data = Sha256::digest(rp_id.as_bytes()).to_vec();
            data.push(flags);
            data.extend(self.counter.to_be_bytes());
            data
        }

        /// `authenticatorData` of a registration: the fixed start, then the
        /// AAGUID and the credential ID.
        fn attested(&self, rp_id: &str, flags: u8) -> Vec<u8> {
            let mut data = self.data(rp_id, flags);
            data.extend([0u8; 16]);
            data.extend((self.credential_id.len() as u16).to_be_bytes());
            data.extend(&self.credential_id);
            data
        }

        fn register(&self, challenge: &[u8]) -> Registration {
            let data = self.attested("remotehub.example.com", 0x41);
            let spki = self.key.verifying_key().to_public_key_der().unwrap();
            serde_json::from_value(json!({
                "rawId": b64(&self.credential_id),
                "response": {
                    "clientDataJSON": b64(&client_data("webauthn.create", challenge, &rp().origin)),
                    "authenticatorData": b64(&data),
                    "publicKey": b64(spki.as_bytes()),
                    "publicKeyAlgorithm": ES256,
                },
            }))
            .unwrap()
        }

        fn sign(&mut self, challenge: &[u8], origin: &str) -> Assertion {
            self.counter = self.counter.wrapping_add(1);
            let data = self.data("remotehub.example.com", 0x01);
            let client = client_data("webauthn.get", challenge, origin);
            let signed = [data.as_slice(), &Sha256::digest(&client)].concat();
            let signature: Signature = self.key.sign(&signed);
            serde_json::from_value(json!({
                "rawId": b64(&self.credential_id),
                "response": {
                    "clientDataJSON": b64(&client),
                    "authenticatorData": b64(&data),
                    "signature": b64(signature.to_der().as_bytes()),
                },
            }))
            .unwrap()
        }
    }

    #[test]
    fn the_relying_party_is_the_host_of_the_public_origin() {
        assert_eq!(
            RelyingParty::of("http://localhost:5180/"),
            RelyingParty {
                id: "localhost".into(),
                origin: "http://localhost:5180".into()
            }
        );
    }

    #[test]
    fn a_registered_key_signs_its_challenges_and_nothing_else() {
        let mut device = Authenticator::new();
        let key = register(
            &rp(),
            b"register-challenge",
            &device.register(b"register-challenge"),
        )
        .unwrap();
        assert_eq!(key.credential_id, device.credential_id);
        assert_eq!(
            register(
                &rp(),
                b"another-challenge",
                &device.register(b"register-challenge")
            ),
            Err(WebauthnError::Mismatch)
        );

        let answer = device.sign(b"sign-in", &rp().origin);
        assert_eq!(
            verify(&rp(), b"sign-in", &key.public_key, 0, &answer),
            Ok(1)
        );
        // The same answer for another challenge, from another origin, or
        // replayed after the counter moved on.
        assert_eq!(
            verify(&rp(), b"other", &key.public_key, 0, &answer),
            Err(WebauthnError::Mismatch)
        );
        let phished = device.sign(b"sign-in", "https://remotehub.example.org");
        assert_eq!(
            verify(&rp(), b"sign-in", &key.public_key, 1, &phished),
            Err(WebauthnError::Mismatch)
        );
        let again = device.sign(b"sign-in", &rp().origin);
        assert_eq!(
            verify(&rp(), b"sign-in", &key.public_key, 5, &again),
            Err(WebauthnError::Counter)
        );
        // A counter that stands still, or drops to 0 once it counted.
        assert_eq!(device.counter, 3);
        assert_eq!(
            verify(&rp(), b"sign-in", &key.public_key, 3, &again),
            Err(WebauthnError::Counter)
        );
        device.counter = u32::MAX;
        let wrapped = device.sign(b"sign-in", &rp().origin);
        assert_eq!(device.counter, 0);
        assert_eq!(
            verify(&rp(), b"sign-in", &key.public_key, 3, &wrapped),
            Err(WebauthnError::Counter)
        );
        // Another key's signature does not verify.
        let other = Authenticator {
            key: SigningKey::from_slice(&[8u8; 32]).unwrap(),
            ..Authenticator::new()
        };
        let mut other = other;
        let forged = other.sign(b"sign-in", &rp().origin);
        assert_eq!(
            verify(&rp(), b"sign-in", &key.public_key, 0, &forged),
            Err(WebauthnError::Signature)
        );
    }

    #[test]
    fn a_key_for_another_site_or_without_presence_is_refused() {
        let device = Authenticator::new();
        let mut elsewhere = device.register(b"c");
        elsewhere.response.authenticator_data = b64(&device.attested("evil.example.com", 0x41));
        assert_eq!(
            register(&rp(), b"c", &elsewhere),
            Err(WebauthnError::Mismatch)
        );

        let mut absent = device.register(b"c");
        absent.response.authenticator_data = b64(&device.attested("remotehub.example.com", 0x40));
        assert_eq!(
            register(&rp(), b"c", &absent),
            Err(WebauthnError::NotPresent)
        );

        let mut rsa = device.register(b"c");
        rsa.response.algorithm = -257;
        assert_eq!(register(&rp(), b"c", &rsa), Err(WebauthnError::Algorithm));
    }
}
