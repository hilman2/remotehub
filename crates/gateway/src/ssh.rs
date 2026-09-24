//! SSH on the server (ADR 0003): russh connects, checks the host key,
//! signs in and opens a shell with a pseudo-terminal. Only terminal bytes
//! leave this module; the password never does.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle, Msg};
use russh::keys::{
    Certificate, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate,
};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

/// How to sign in.
pub enum SshAuth<'a> {
    Password(&'a SecretString),
    Key(&'a SshKey),
}

/// A private key, decrypted and ready to sign in, optionally with an OpenSSH
/// user certificate. Built only on the server; never serialised.
pub struct SshKey {
    key: Arc<PrivateKey>,
    certificate: Option<Certificate>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("not a private key in OpenSSH or PEM format")]
    InvalidKey,
    #[error("the key needs its passphrase")]
    PassphraseRequired,
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("not an OpenSSH user certificate")]
    InvalidCertificate,
    #[error("the certificate belongs to another key")]
    CertificateMismatch,
}

impl SshKey {
    /// Reads a private key (optionally encrypted) and an optional
    /// certificate, and checks that they belong together.
    pub fn parse(
        private_key: &str,
        passphrase: Option<&str>,
        certificate: Option<&str>,
    ) -> Result<SshKey, KeyError> {
        let passphrase = passphrase.filter(|p| !p.is_empty());
        let key = match russh::keys::decode_secret_key(private_key.trim(), None) {
            Ok(key) => key,
            Err(_) if !looks_like_private_key(private_key) => return Err(KeyError::InvalidKey),
            Err(_) => match passphrase {
                None => return Err(KeyError::PassphraseRequired),
                Some(passphrase) => {
                    russh::keys::decode_secret_key(private_key.trim(), Some(passphrase))
                        .map_err(|_| KeyError::WrongPassphrase)?
                }
            },
        };
        let certificate = certificate
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| Certificate::from_openssh(c).map_err(|_| KeyError::InvalidCertificate))
            .transpose()?;
        if let Some(certificate) = &certificate {
            if certificate.cert_type() != russh::keys::ssh_key::certificate::CertType::User {
                return Err(KeyError::InvalidCertificate);
            }
            if certificate.public_key() != key.public_key().key_data() {
                return Err(KeyError::CertificateMismatch);
            }
        }
        Ok(SshKey {
            key: Arc::new(key),
            certificate,
        })
    }

    /// e.g. `ssh-ed25519`
    pub fn algorithm(&self) -> String {
        self.key.algorithm().to_string()
    }

    /// SHA-256 fingerprint of the public key.
    pub fn fingerprint(&self) -> String {
        self.key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string()
    }

    pub fn has_certificate(&self) -> bool {
        self.certificate.is_some()
    }
}

/// An unreadable key that at least has the armour of one is probably
/// encrypted rather than garbage.
fn looks_like_private_key(text: &str) -> bool {
    let text = text.trim();
    text.starts_with("-----BEGIN") && text.contains("PRIVATE KEY-----")
}

/// Where and as whom to connect.
pub struct SshTarget<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub auth: SshAuth<'a>,
    /// The host key pinned at the first connection (OpenSSH format); `None`
    /// trusts the key presented now (trust on first use).
    pub pinned_host_key: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u32,
    pub rows: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SshError {
    #[error("cannot reach {0}")]
    Unreachable(String),
    #[error("the host key changed: pinned {expected}, presented {presented}")]
    HostKeyChanged { expected: String, presented: String },
    #[error("the target refused the credentials")]
    AuthenticationFailed,
    #[error("timed out")]
    Timeout,
    #[error("SSH protocol error: {0}")]
    Protocol(String),
}

/// What the target sends.
#[derive(Debug, PartialEq, Eq)]
pub enum Output {
    /// Terminal output (stdout and stderr share the PTY).
    Data(Vec<u8>),
    /// The shell ended; the exit status if the server sent one.
    Exit(Option<u32>),
}

/// An open shell on the target.
pub struct SshSession {
    /// The presented host key in OpenSSH format, to pin after the first use.
    pub host_key: String,
    /// `SHA256:…`, as `ssh-keygen -lf` shows it.
    pub host_key_fingerprint: String,
    reader: ChannelReadHalf,
    writer: ChannelWriteHalf<Msg>,
    handle: Handle<Client>,
    exit_status: Option<u32>,
    finished: bool,
}

/// SHA-256 fingerprint of a key in OpenSSH format.
pub fn fingerprint(openssh: &str) -> Option<String> {
    PublicKey::from_openssh(openssh)
        .ok()
        .map(|key| key.fingerprint(HashAlg::Sha256).to_string())
}

struct Client {
    pinned: Option<PublicKey>,
    presented: Arc<Mutex<Option<PublicKey>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Host certificates arrive with the built-in SSH CA (backlog).
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Ok(false);
        };
        *self.presented.lock().expect("host key lock") = Some(key.clone());
        Ok(match &self.pinned {
            None => true,
            Some(pinned) => pinned.key_data() == key.key_data(),
        })
    }
}

/// Connects, checks the host key, signs in with the password and opens a
/// shell with a pseudo-terminal of the given size.
pub async fn open(
    target: SshTarget<'_>,
    size: Size,
    timeout: Duration,
) -> Result<SshSession, SshError> {
    tokio::time::timeout(timeout, open_inner(target, size))
        .await
        .map_err(|_| SshError::Timeout)?
}

async fn open_inner(target: SshTarget<'_>, size: Size) -> Result<SshSession, SshError> {
    let pinned = target
        .pinned_host_key
        .map(PublicKey::from_openssh)
        .transpose()
        .map_err(|e| SshError::Protocol(format!("pinned host key is invalid: {e}")))?;
    let presented = Arc::new(Mutex::new(None));
    let handler = Client {
        pinned: pinned.clone(),
        presented: presented.clone(),
    };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });

    let address = format!("{}:{}", target.host, target.port);
    let connected = client::connect(config, (target.host, target.port), handler).await;
    let presented_key = presented.lock().expect("host key lock").clone();
    let mut handle = match connected {
        Ok(handle) => handle,
        Err(error) => {
            return Err(match (&pinned, &presented_key) {
                (Some(pinned), Some(presented)) if pinned.key_data() != presented.key_data() => {
                    SshError::HostKeyChanged {
                        expected: pinned.fingerprint(HashAlg::Sha256).to_string(),
                        presented: presented.fingerprint(HashAlg::Sha256).to_string(),
                    }
                }
                (_, None) => SshError::Unreachable(format!("{address}: {error}")),
                _ => SshError::Protocol(error.to_string()),
            });
        }
    };
    let host_key = presented_key.ok_or_else(|| SshError::Protocol("no host key".into()))?;

    let auth = match target.auth {
        SshAuth::Password(password) => {
            handle
                .authenticate_password(target.username, password.expose_secret())
                .await
        }
        SshAuth::Key(SshKey {
            key,
            certificate: Some(certificate),
        }) => {
            handle
                .authenticate_openssh_cert(target.username, key.clone(), certificate.clone())
                .await
        }
        SshAuth::Key(SshKey {
            key,
            certificate: None,
        }) => {
            // RSA keys sign with the best hash the server accepts.
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .ok()
                .flatten()
                .flatten();
            handle
                .authenticate_publickey(
                    target.username,
                    PrivateKeyWithHashAlg::new(key.clone(), hash),
                )
                .await
        }
    }
    .map_err(|e| SshError::Protocol(e.to_string()))?;
    if !auth.success() {
        return Err(SshError::AuthenticationFailed);
    }

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| SshError::Protocol(e.to_string()))?;
    channel
        .request_pty(false, "xterm-256color", size.cols, size.rows, 0, 0, &[])
        .await
        .map_err(|e| SshError::Protocol(e.to_string()))?;
    channel
        .request_shell(false)
        .await
        .map_err(|e| SshError::Protocol(e.to_string()))?;
    let (reader, writer) = channel.split();

    Ok(SshSession {
        host_key: host_key
            .to_openssh()
            .map_err(|e| SshError::Protocol(e.to_string()))?,
        host_key_fingerprint: host_key.fingerprint(HashAlg::Sha256).to_string(),
        reader,
        writer,
        handle,
        exit_status: None,
        finished: false,
    })
}

impl SshSession {
    /// Keystrokes and pasted text for the shell.
    pub async fn send(&self, data: &[u8]) -> Result<(), SshError> {
        self.writer
            .data(data)
            .await
            .map_err(|e| SshError::Protocol(e.to_string()))
    }

    pub async fn resize(&self, size: Size) -> Result<(), SshError> {
        self.writer
            .window_change(size.cols, size.rows, 0, 0)
            .await
            .map_err(|e| SshError::Protocol(e.to_string()))
    }

    /// The next output; `Exit` once, then `None`.
    pub async fn next(&mut self) -> Option<Output> {
        if self.finished {
            return None;
        }
        loop {
            match self.reader.wait().await {
                Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                    return Some(Output::Data(data.to_vec()));
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    self.exit_status = Some(exit_status)
                }
                // OpenSSH sends EOF before the exit status; the channel ends with Close.
                Some(ChannelMsg::Close) | None => {
                    self.finished = true;
                    return Some(Output::Exit(self.exit_status));
                }
                Some(_) => {}
            }
        }
    }

    /// Ends the shell and the connection.
    pub async fn close(self) {
        let _ = self.writer.eof().await;
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }
}
