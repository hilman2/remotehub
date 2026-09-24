//! remotehub as an SSH certificate authority: targets trust its public key
//! (`TrustedUserCAKeys`), and every connection signs in with a fresh key and
//! a user certificate that is valid for minutes. No password or key is
//! stored for such a device.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::certificate::{Builder, CertType};
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::{Certificate, PrivateKey};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::ssh::SshKey;

/// How far a certificate's start lies in the past, for targets whose clock
/// runs a little behind.
const CLOCK_SKEW: Duration = Duration::from_secs(60);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaError {
    #[error("not an Ed25519 private key in OpenSSH format")]
    InvalidKey,
    #[error("cannot sign the certificate: {0}")]
    Signing(String),
}

/// The CA's private key.
pub struct SshCa {
    key: PrivateKey,
}

impl std::fmt::Debug for SshCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshCa")
            .field("public_key", &self.public_key())
            .finish_non_exhaustive()
    }
}

impl SshCa {
    /// A new CA key in OpenSSH format, for the key file.
    pub fn generate() -> Zeroizing<String> {
        let key = PrivateKey::from(random_ed25519());
        key.to_openssh(LineEnding::LF)
            .expect("an Ed25519 key always encodes")
    }

    /// Reads the CA key: an unencrypted Ed25519 key in OpenSSH format.
    pub fn from_openssh(text: &str) -> Result<SshCa, CaError> {
        let key = PrivateKey::from_openssh(text.trim()).map_err(|_| CaError::InvalidKey)?;
        if key.is_encrypted() || !matches!(key.key_data(), KeypairData::Ed25519(_)) {
            return Err(CaError::InvalidKey);
        }
        Ok(SshCa { key })
    }

    /// The public key in the form of `TrustedUserCAKeys` and
    /// `authorized_keys`: `ssh-ed25519 AAAA… remotehub`.
    pub fn public_key(&self) -> String {
        let mut public = self.key.public_key().clone();
        public.set_comment("remotehub");
        public.to_openssh().expect("an Ed25519 key always encodes")
    }

    /// A fresh key with a user certificate for `principal`, valid for
    /// `validity` from now. `key_id` shows up in the target's log.
    pub fn issue(
        &self,
        principal: &str,
        key_id: &str,
        validity: Duration,
    ) -> Result<SshKey, CaError> {
        let key = PrivateKey::from(random_ed25519());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970");
        let certificate = self.sign(&key, principal, key_id, now, validity)?;
        Ok(SshKey::with_certificate(Arc::new(key), certificate))
    }

    fn sign(
        &self,
        key: &PrivateKey,
        principal: &str,
        key_id: &str,
        now: Duration,
        validity: Duration,
    ) -> Result<Certificate, CaError> {
        let signing = |e: russh::keys::ssh_key::Error| CaError::Signing(e.to_string());
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).expect("the operating system provides randomness");
        let mut builder = Builder::new(
            nonce.to_vec(),
            key.public_key().key_data().clone(),
            now.saturating_sub(CLOCK_SKEW).as_secs(),
            (now + validity).as_secs(),
        )
        .map_err(signing)?;
        builder
            .cert_type(CertType::User)
            .map_err(signing)?
            .key_id(key_id)
            .map_err(signing)?
            .valid_principal(principal)
            .map_err(signing)?
            .extension("permit-pty", "")
            .map_err(signing)?;
        builder.sign(&self.key).map_err(signing)
    }
}

fn random_ed25519() -> Ed25519Keypair {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut_slice()).expect("the operating system provides randomness");
    Ed25519Keypair::from_seed(&seed)
}

#[cfg(test)]
mod tests {
    use russh::keys::ssh_key::PublicKey;

    use super::*;

    #[test]
    fn certificates_name_the_principal_and_expire_soon() {
        let ca = SshCa::from_openssh(&SshCa::generate()).unwrap();
        let key = ca
            .issue("alice", "remotehub alice", Duration::from_secs(300))
            .unwrap();
        let certificate = key.certificate().unwrap();
        let ca_key = PublicKey::from_openssh(&ca.public_key()).unwrap();

        assert_eq!(certificate.cert_type(), CertType::User);
        assert_eq!(certificate.valid_principals(), ["alice"]);
        assert_eq!(certificate.key_id(), "remotehub alice");
        assert_eq!(certificate.signature_key(), ca_key.key_data());
        assert!(certificate.extensions().contains_key("permit-pty"));
        let lifetime = certificate.valid_before() - certificate.valid_after();
        assert_eq!(lifetime, 300 + CLOCK_SKEW.as_secs());
        // The certificate verifies against the CA, and only against it.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        certificate
            .validate_at(now, [&ca_key.fingerprint(Default::default())])
            .unwrap();
        let other = SshCa::from_openssh(&SshCa::generate()).unwrap();
        let other_key = PublicKey::from_openssh(&other.public_key()).unwrap();
        assert!(
            certificate
                .validate_at(now, [&other_key.fingerprint(Default::default())])
                .is_err()
        );
        assert!(
            certificate
                .validate_at(now + 301, [&ca_key.fingerprint(Default::default())])
                .is_err()
        );
    }

    #[test]
    fn every_connection_gets_its_own_key() {
        let ca = SshCa::from_openssh(&SshCa::generate()).unwrap();
        let first = ca.issue("alice", "a", Duration::from_secs(60)).unwrap();
        let second = ca.issue("alice", "b", Duration::from_secs(60)).unwrap();
        assert_ne!(first.fingerprint(), second.fingerprint());
    }

    #[test]
    fn only_unencrypted_ed25519_keys_are_cas() {
        assert_eq!(
            SshCa::from_openssh("not a key").err(),
            Some(CaError::InvalidKey)
        );
        let tester = include_str!("../../../deploy/testlab/ssh/tester_ed25519_passphrase");
        assert_eq!(SshCa::from_openssh(tester).err(), Some(CaError::InvalidKey));
    }
}
