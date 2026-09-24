//! RDP and VNC through guacd against the test lab's desktop target
//! (deploy/testlab/desktop). Needs `REMOTEHUB_TEST_GUACD` and
//! `REMOTEHUB_TEST_DESKTOP_HOST`; run with `cargo nextest run --run-ignored only`.

use std::time::Duration;

use remotehub_gateway::guacamole::{self, Connection, GuacError, Handshake};

/// The lab certificate's fingerprint (deploy/testlab/desktop/README.md).
const LAB_CERTIFICATE: &str = "sha256:C1:E8:6D:13:4E:8D:B7:A5:D2:72:01:8F:93:8F:C4:44:EC:E4:C0:D5:97:C8:00:EF:25:24:BB:76:22:0B:DF:CD";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} points to the test lab"))
}

async fn open(protocol: &str, parameters: &[(&str, &str)]) -> Result<Connection, GuacError> {
    open_within(protocol, parameters, Duration::from_secs(10)).await
}

async fn open_within(
    protocol: &str,
    parameters: &[(&str, &str)],
    timeout: Duration,
) -> Result<Connection, GuacError> {
    guacamole::open(
        &env("REMOTEHUB_TEST_GUACD"),
        &Handshake {
            protocol,
            parameters,
            width: 1024,
            height: 768,
            dpi: 96,
            timezone: Some("Europe/Berlin"),
        },
        timeout,
    )
    .await
}

/// guacd draws a blank frame before it reaches the target; an image only
/// comes from the target.
const FRAME: &str = "3.img,";

/// Everything guacd sends until `needle` appears (or ten seconds pass).
async fn read_until(connection: &mut Connection, needle: &str) -> String {
    let mut seen = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while let Ok(Some(text)) = connection.receive().await {
            seen.push_str(&text);
            if seen.contains(needle) {
                break;
            }
        }
    })
    .await;
    seen
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn rdp_shows_the_desktop_with_a_pinned_certificate() {
    let host = env("REMOTEHUB_TEST_DESKTOP_HOST");
    let mut connection = open(
        "rdp",
        &[
            ("hostname", &host),
            ("port", "3389"),
            ("username", "tester"),
            ("password", "Tester-Passw0rd!"),
            ("security", "tls"),
            ("cert-fingerprints", LAB_CERTIFICATE),
        ],
    )
    .await
    .unwrap();
    assert!(connection.id.starts_with('$'), "{}", connection.id);
    let seen = read_until(&mut connection, FRAME).await;
    assert!(seen.contains(FRAME), "no picture: {seen:.300}");
    assert!(!seen.contains("5.error,"), "{seen:.300}");
    connection.close().await;
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn rdp_refuses_a_certificate_that_is_not_pinned() {
    let host = env("REMOTEHUB_TEST_DESKTOP_HOST");
    let other = LAB_CERTIFICATE.replace("C1:E8", "00:00");
    let mut connection = open(
        "rdp",
        &[
            ("hostname", &host),
            ("port", "3389"),
            ("username", "tester"),
            ("password", "Tester-Passw0rd!"),
            ("security", "tls"),
            ("cert-fingerprints", &other),
        ],
    )
    .await
    .unwrap();
    let seen = read_until(&mut connection, "5.error,").await;
    assert!(seen.contains("5.error,"), "{seen:.300}");
    assert!(!seen.contains(FRAME), "a picture arrived: {seen:.300}");
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn vnc_shows_the_desktop_and_refuses_a_wrong_password() {
    let host = env("REMOTEHUB_TEST_DESKTOP_HOST");
    let mut connection = open(
        "vnc",
        &[
            ("hostname", &host),
            ("port", "5900"),
            ("password", "Vnc-Pw1!"),
        ],
    )
    .await
    .unwrap();
    let seen = read_until(&mut connection, FRAME).await;
    assert!(seen.contains(FRAME), "no picture: {seen:.300}");
    connection.close().await;

    let mut connection = open(
        "vnc",
        &[("hostname", &host), ("port", "5900"), ("password", "wrong")],
    )
    .await
    .unwrap();
    let seen = read_until(&mut connection, "5.error,").await;
    assert!(seen.contains("5.error,"), "{seen:.300}");
    assert!(!seen.contains(FRAME), "a picture arrived: {seen:.300}");
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn guacd_offers_neither_ssh_nor_telnet() {
    for protocol in ["ssh", "telnet"] {
        // guacd does not answer at all for a protocol it lacks.
        let result = open_within(
            protocol,
            &[("hostname", "localhost")],
            Duration::from_secs(2),
        )
        .await;
        assert!(result.is_err(), "{protocol} is available");
    }
}
