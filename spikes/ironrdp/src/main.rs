//! The IronRDP spike of ADR 0006: connects to an RDP server, decodes the
//! desktop until it has been quiet for five seconds, and reports what it
//! measured. Against the test lab (`docker compose -f deploy/compose.dev.yml`
//! network):
//!
//!   AUTOLOGON=1 ironrdp-spike desktop-target 3389 tester 'Tester-Passw0rd!'
//!
//! It accepts any server certificate: it measures, it protects nothing.

use std::io::Write as _;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use ironrdp::connector::{self, ConnectionResult, Credentials};
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageBuilder, ActiveStageOutput};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use sspi::network_client::reqwest_network_client::ReqwestNetworkClient;
use tokio_rustls::rustls;

type Upgraded = ironrdp_blocking::Framed<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (host, port, user, password) = (&args[1], args[2].parse::<u16>()?, &args[3], &args[4]);
    let started = Instant::now();
    let config = connector::Config {
        credentials: Credentials::UsernamePassword {
            username: user.clone(),
            password: password.clone(),
        },
        domain: None,
        enable_tls: true,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: 1024,
            height: 768,
        },
        bitmap: None,
        client_build: 0,
        client_name: "remotehub-spike".to_owned(),
        client_dir: String::new(),
        platform: MajorPlatformType::UNIX,
        enable_server_pointer: false,
        request_data: None,
        // Without NLA (xrdp in the lab), only autologon passes the
        // credentials on; otherwise the server shows its own sign-in screen.
        autologon: std::env::var("AUTOLOGON").is_ok(),
        enable_audio_playback: false,
        compression_type: None,
        pointer_software_rendering: true,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    };
    let (result, mut framed) = connect(config, host, port).context("connect")?;
    println!(
        "connected after {:?}, desktop {}x{}",
        started.elapsed(),
        result.desktop_size.width,
        result.desktop_size.height
    );

    let mut image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        result.desktop_size.width,
        result.desktop_size.height,
    );
    let (frames, bytes, first_update) = active(result, &mut framed, &mut image)?;
    let data = image.data();
    let at = |x: usize, y: usize| {
        let i = (y * usize::from(image.width()) + x) * 4;
        (data[i], data[i + 1], data[i + 2])
    };
    println!(
        "frames {frames}, bytes {bytes}, first graphics update after {first_update:?}, pixel(4,4) {:?}, pixel(500,700) {:?}",
        at(4, 4),
        at(500, 700)
    );
    Ok(())
}

fn connect(
    config: connector::Config,
    host: &str,
    port: u16,
) -> anyhow::Result<(ConnectionResult, Upgraded)> {
    use std::net::ToSocketAddrs as _;
    let address = (host, port).to_socket_addrs()?.next().context("address")?;
    let tcp = TcpStream::connect(address)?;
    // The read timeout ends the active stage once the server is quiet.
    tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
    let client_addr = tcp.local_addr()?;
    let mut framed = ironrdp_blocking::Framed::new(tcp);
    let mut connector = connector::ClientConnector::new(config, client_addr);
    let should_upgrade =
        ironrdp_blocking::connect_begin(&mut framed, &mut connector).context("begin")?;
    let stream = framed.into_inner_no_leftover();
    let (tls, public_key) = tls_upgrade(stream, host.to_owned())?;
    let upgraded = ironrdp_blocking::mark_as_upgraded(should_upgrade, &mut connector);
    let mut framed = ironrdp_blocking::Framed::new(tls);
    let result = ironrdp_blocking::connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut ReqwestNetworkClient,
        host.to_owned().into(),
        public_key,
        None,
    )
    .context("finalize")?;
    Ok((result, framed))
}

/// Decodes into `image` until the server is quiet: frames, bytes, and the
/// time to the first graphics update.
fn active(
    result: ConnectionResult,
    framed: &mut Upgraded,
    image: &mut DecodedImage,
) -> anyhow::Result<(usize, usize, Duration)> {
    let mut stage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();
    let started = Instant::now();
    let (mut frames, mut bytes, mut first) = (0, 0, None);
    loop {
        let (action, payload) = match framed.read_pdu() {
            Ok(frame) => frame,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(anyhow::Error::new(e).context("read")),
        };
        frames += 1;
        bytes += payload.len();
        for out in stage.process(image, action, &payload)? {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => framed.write_all(&frame)?,
                ActiveStageOutput::GraphicsUpdate(_) if first.is_none() => {
                    first = Some(started.elapsed());
                }
                ActiveStageOutput::Terminate(_) => {
                    return Ok((frames, bytes, first.unwrap_or_default()));
                }
                _ => {}
            }
        }
    }
    Ok((frames, bytes, first.unwrap_or_default()))
}

fn tls_upgrade(
    stream: TcpStream,
    name: String,
) -> anyhow::Result<(rustls::StreamOwned<rustls::ClientConnection, TcpStream>, Vec<u8>)> {
    let mut config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(danger::Accept))
        .with_no_client_auth();
    // CredSSP does not support TLS session resumption.
    config.resumption = rustls::client::Resumption::disabled();
    let client = rustls::ClientConnection::new(std::sync::Arc::new(config), name.try_into()?)?;
    let mut tls = rustls::StreamOwned::new(client, stream);
    tls.flush()?;
    let cert = tls
        .conn
        .peer_certificates()
        .and_then(|c| c.first())
        .context("certificate")?;
    use x509_cert::der::Decode as _;
    let cert = x509_cert::Certificate::from_der(cert)?;
    let key = cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .context("key")?
        .to_owned();
    Ok((tls, key))
}

mod danger {
    use tokio_rustls::rustls::client::danger::{
        HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
    };
    use tokio_rustls::rustls::{DigitallySignedStruct, Error, SignatureScheme, pki_types};

    #[derive(Debug)]
    pub(super) struct Accept;

    impl ServerCertVerifier for Accept {
        fn verify_server_cert(
            &self,
            _: &pki_types::CertificateDer<'_>,
            _: &[pki_types::CertificateDer<'_>],
            _: &pki_types::ServerName<'_>,
            _: &[u8],
            _: pki_types::UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
            ]
        }
    }
}
