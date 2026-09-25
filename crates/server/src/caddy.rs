//! Caddy of the ops package (#142, #146), seen from remotehub: its admin API
//! on a Unix socket that only Caddy and remotehub share, and a directory
//! both mount, where remotehub writes the snippet `(remotehub_tls)` that
//! says where Caddy's certificate comes from, and a certificate of your own.
//!
//! remotehub writes the files when it starts, before Caddy starts. A change
//! writes them and then has Caddy load its Caddyfile again, which imports
//! them. Caddy keeps its old configuration if the new one does not load;
//! remotehub then puts its files back too.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use tokio::net::{TcpStream, UnixStream};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

const TIMEOUT: Duration = Duration::from_secs(15);
const SNIPPET: &str = "tls.caddy";
const CERTIFICATE: &str = "certificate.pem";
const KEY: &str = "key.pem";
/// Where Caddy sees the directory remotehub writes into.
const CADDY_SITES: &str = "/etc/caddy/remotehub";

/// Where remotehub finds Caddy; from `REMOTEHUB_CADDY_*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaddyConfig {
    /// The admin API's Unix socket.
    pub socket: PathBuf,
    /// The directory Caddy imports from, as remotehub mounts it.
    pub sites: PathBuf,
    /// Caddy's Caddyfile, to load again after a change.
    pub caddyfile: PathBuf,
    /// Where Caddy serves HTTPS, e.g. `caddy:443`, to see its certificate.
    pub address: String,
}

/// Where Caddy's certificate comes from.
pub enum Tls {
    /// Let's Encrypt if the internet reaches the host, else Caddy's own CA.
    Automatic,
    /// A certificate of your own, as PEM: the chain, and its key.
    Own {
        chain: String,
        key: Zeroizing<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CaddyError {
    #[error("Caddy is not reachable: {0}")]
    Unreachable(String),
    /// Caddy refused the change and keeps its configuration as it was.
    #[error("Caddy refused: {0}")]
    Refused(String),
    #[error("cannot write Caddy's files: {0}")]
    Files(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct Caddy {
    config: CaddyConfig,
    /// The host people open, which Caddy serves.
    host: String,
}

impl Caddy {
    pub fn new(config: CaddyConfig, host: String) -> Self {
        Caddy { config, host }
    }

    /// Whether Caddy of the package runs: its socket is there. Behind a
    /// reverse proxy of your own, it is not.
    pub fn runs(&self) -> bool {
        self.config.socket.exists()
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> Result<(u16, Bytes), CaddyError> {
        let unreachable = |e: &dyn std::fmt::Display| CaddyError::Unreachable(e.to_string());
        let exchange = async {
            let stream = UnixStream::connect(&self.config.socket)
                .await
                .map_err(|e| unreachable(&e))?;
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake(TokioIo::new(stream))
                    .await
                    .map_err(|e| unreachable(&e))?;
            tokio::spawn(connection);
            let mut request = Request::builder()
                .method(method)
                .uri(path)
                // The admin API refuses requests without a host it knows.
                .header(hyper::header::HOST, "localhost");
            if let Some(content_type) = content_type {
                request = request.header(hyper::header::CONTENT_TYPE, content_type);
            }
            let request = request
                .body(Full::new(Bytes::from(body)))
                .map_err(|e| unreachable(&e))?;
            let response = sender
                .send_request(request)
                .await
                .map_err(|e| unreachable(&e))?;
            let status = response.status().as_u16();
            let body = response
                .into_body()
                .collect()
                .await
                .map_err(|e| unreachable(&e))?
                .to_bytes();
            Ok::<_, CaddyError>((status, body))
        };
        timeout(TIMEOUT, exchange)
            .await
            .map_err(|_| CaddyError::Unreachable("no answer in time".into()))?
    }

    /// The root certificate of Caddy's own CA, as PEM.
    pub async fn root(&self) -> Result<String, CaddyError> {
        let (status, body) = self
            .request(Method::GET, "/pki/ca/local", None, Vec::new())
            .await?;
        if status != 200 {
            return Err(CaddyError::Refused(
                String::from_utf8_lossy(&body).into_owned(),
            ));
        }
        let ca: serde_json::Value =
            serde_json::from_slice(&body).map_err(|e| CaddyError::Refused(e.to_string()))?;
        ca["root_certificate"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| CaddyError::Refused("no root certificate".into()))
    }

    /// Makes Caddy serve with `tls`. If Caddy refuses, its configuration
    /// and remotehub's files stay as they were.
    pub async fn serve(&self, tls: &Tls) -> Result<(), CaddyError> {
        let sites = &self.config.sites;
        let before = Files::read(sites);
        write_files(sites, tls)?;
        match self.load().await {
            Ok(()) => Ok(()),
            Err(error) => {
                before.restore(sites)?;
                Err(error)
            }
        }
    }

    /// Writes the files for `tls` without telling Caddy: at startup, before
    /// Caddy starts and imports them.
    pub fn prepare(&self, tls: &Tls) -> Result<(), CaddyError> {
        Ok(write_files(&self.config.sites, tls)?)
    }

    /// Has Caddy load its Caddyfile again, which imports remotehub's files.
    async fn load(&self) -> Result<(), CaddyError> {
        let caddyfile = tokio::fs::read(&self.config.caddyfile).await?;
        let (status, body) = self
            .request(Method::POST, "/load", Some("text/caddyfile"), caddyfile)
            .await?;
        if status == 200 {
            Ok(())
        } else {
            Err(CaddyError::Refused(
                String::from_utf8_lossy(&body).into_owned(),
            ))
        }
    }

    /// The chain Caddy presents for the host right now, its own first.
    pub async fn current(&self) -> Result<Vec<CertificateDer<'static>>, CaddyError> {
        let unreachable = |e: &dyn std::fmt::Display| CaddyError::Unreachable(e.to_string());
        let recorder = Arc::new(Recorder::default());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| unreachable(&e))?
            .dangerous()
            .with_custom_certificate_verifier(recorder.clone())
            .with_no_client_auth();
        let name = ServerName::try_from(self.host.clone()).map_err(|e| unreachable(&e))?;
        let handshake = async {
            let stream = TcpStream::connect(&self.config.address)
                .await
                .map_err(|e| unreachable(&e))?;
            TlsConnector::from(Arc::new(config))
                .connect(name, stream)
                .await
                .map_err(|e| unreachable(&e))
        };
        timeout(TIMEOUT, handshake)
            .await
            .map_err(|_| CaddyError::Unreachable("no answer in time".into()))??;
        Ok(recorder.chain.lock().expect("chain lock").clone())
    }
}

/// The snippet that says where Caddy's certificate comes from.
fn snippet(tls: &Tls) -> String {
    let rule = match tls {
        Tls::Automatic => "\ttls {\n\t\tissuer acme\n\t\tissuer internal\n\t}\n".to_owned(),
        Tls::Own { .. } => format!("\ttls {CADDY_SITES}/{CERTIFICATE} {CADDY_SITES}/{KEY}\n"),
    };
    format!(
        "# Where Caddy's certificate comes from. remotehub writes this file from\n\
         # Settings -> Certificate (#146); a change here is overwritten there.\n\
         (remotehub_tls) {{\n{rule}}}\n"
    )
}

fn write_files(sites: &Path, tls: &Tls) -> std::io::Result<()> {
    match tls {
        Tls::Automatic => {
            for file in [CERTIFICATE, KEY] {
                match std::fs::remove_file(sites.join(file)) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                        return Err(error);
                    }
                    _ => {}
                }
            }
        }
        Tls::Own { chain, key } => {
            write(sites, CERTIFICATE, chain.as_bytes())?;
            write(sites, KEY, key.as_bytes())?;
        }
    }
    write(sites, SNIPPET, snippet(tls).as_bytes())
}

/// Readable by the group: Caddy runs as root without capabilities and reads
/// the files as the group they take from the directory, which init.sh gives
/// to root. Others read nothing, the key least of all.
const MODE: u32 = 0o640;

/// Writes `name` in `dir` all at once: a new file, renamed into place.
fn write(dir: &Path, name: &str, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = dir.join(format!(".{name}.new"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(MODE)
        .open(&temporary)?;
    file.write_all(content)?;
    file.sync_all()?;
    std::fs::rename(temporary, dir.join(name))
}

/// remotehub's files as they were before a change, to put back.
struct Files(Vec<(&'static str, Option<Zeroizing<Vec<u8>>>)>);

impl Files {
    fn read(dir: &Path) -> Self {
        Files(
            [SNIPPET, CERTIFICATE, KEY]
                .into_iter()
                .map(|name| (name, std::fs::read(dir.join(name)).ok().map(Zeroizing::new)))
                .collect(),
        )
    }

    fn restore(&self, dir: &Path) -> std::io::Result<()> {
        for (name, content) in &self.0 {
            match content {
                Some(content) => write(dir, name, content)?,
                None => match std::fs::remove_file(dir.join(name)) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                        return Err(error);
                    }
                    _ => {}
                },
            }
        }
        Ok(())
    }
}

/// Takes any certificate and keeps the chain: only to see what Caddy
/// serves, never to trust it.
#[derive(Debug, Default)]
struct Recorder {
    chain: Mutex<Vec<CertificateDer<'static>>>,
}

impl ServerCertVerifier for Recorder {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.chain.lock().expect("chain lock") = std::iter::once(end_entity)
            .chain(intermediates)
            .map(|c| c.clone().into_owned())
            .collect();
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snippet_names_the_issuers_or_the_files() {
        let automatic = snippet(&Tls::Automatic);
        assert!(automatic.contains("(remotehub_tls) {"));
        assert!(automatic.contains("issuer acme\n\t\tissuer internal"));
        let own = snippet(&Tls::Own {
            chain: String::new(),
            key: Zeroizing::new(String::new()),
        });
        assert!(
            own.contains("tls /etc/caddy/remotehub/certificate.pem /etc/caddy/remotehub/key.pem")
        );
    }

    #[test]
    fn files_go_back_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        write_files(dir.path(), &Tls::Automatic).unwrap();
        let before = Files::read(dir.path());
        let own = Tls::Own {
            chain: "chain".into(),
            key: Zeroizing::new("key".into()),
        };
        write_files(dir.path(), &own).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(KEY)).unwrap(),
            "key"
        );
        before.restore(dir.path()).unwrap();
        assert!(!dir.path().join(KEY).exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(SNIPPET)).unwrap(),
            snippet(&Tls::Automatic)
        );
    }
}
