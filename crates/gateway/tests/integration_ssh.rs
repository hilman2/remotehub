//! SSH against the test lab's target (deploy/testlab/ssh: user `tester`).
//! Needs `REMOTEHUB_TEST_SSH_HOST`; run with `cargo nextest run --run-ignored only`.

use std::time::Duration;

use remotehub_gateway::ssh::{self, Output, Size, SshError, SshSession, SshTarget};
use secrecy::SecretString;

const SIZE: Size = Size { cols: 80, rows: 24 };

fn host() -> String {
    std::env::var("REMOTEHUB_TEST_SSH_HOST")
        .expect("REMOTEHUB_TEST_SSH_HOST points to the test lab")
}

async fn connect(password: &str, pinned: Option<&str>, port: u16) -> Result<SshSession, SshError> {
    let host = host();
    let password = SecretString::from(password.to_owned());
    ssh::open(
        SshTarget {
            host: &host,
            port,
            username: "tester",
            password: &password,
            pinned_host_key: pinned,
        },
        SIZE,
        Duration::from_secs(10),
    )
    .await
}

/// Collects output until `needle` shows up (or ten seconds pass).
async fn read_until(session: &mut SshSession, needle: &str) -> String {
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !seen.contains(needle) {
        match tokio::time::timeout_at(deadline, session.next()).await {
            Ok(Some(Output::Data(data))) => seen.push_str(&String::from_utf8_lossy(&data)),
            Ok(Some(Output::Exit(_)) | None) | Err(_) => break,
        }
    }
    seen
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn runs_commands_in_a_terminal() {
    let mut session = connect("Tester-Passw0rd!", None, 22).await.unwrap();
    assert!(session.host_key.starts_with("ssh-"));
    assert!(session.host_key_fingerprint.starts_with("SHA256:"));
    assert_eq!(
        ssh::fingerprint(&session.host_key).as_deref(),
        Some(session.host_key_fingerprint.as_str())
    );

    session
        .send(b"echo \"me: $(whoami) cols: $(tput cols)\"\n")
        .await
        .unwrap();
    let seen = read_until(&mut session, "cols: 80").await;
    assert!(seen.contains("me: tester cols: 80"), "{seen}");

    session
        .resize(Size {
            cols: 120,
            rows: 40,
        })
        .await
        .unwrap();
    session.send(b"echo \"now $(tput cols)\"\n").await.unwrap();
    assert!(
        read_until(&mut session, "now 120")
            .await
            .contains("now 120")
    );

    session.send(b"exit 3\n").await.unwrap();
    let mut status = None;
    while let Some(output) = session.next().await {
        if let Output::Exit(code) = output {
            status = code;
            break;
        }
    }
    assert_eq!(status, Some(3));
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn a_pinned_host_key_must_match() {
    let first = connect("Tester-Passw0rd!", None, 22).await.unwrap();
    let pinned = first.host_key.clone();
    first.close().await;
    assert!(connect("Tester-Passw0rd!", Some(&pinned), 22).await.is_ok());

    // Another host's key (a fresh ed25519 key) in place of the pinned one.
    let other =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIErqoGI5zlU7vi5Y/fdFH/EJV35jU1dDC5j8WCWzOszR other";
    match connect("Tester-Passw0rd!", Some(other), 22).await {
        Err(SshError::HostKeyChanged {
            expected,
            presented,
        }) => {
            assert_eq!(Some(expected), ssh::fingerprint(other));
            assert_eq!(Some(presented), ssh::fingerprint(&pinned));
        }
        other => panic!("expected a host key change, got {:?}", other.map(|_| ())),
    }
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn wrong_passwords_and_closed_ports_are_reported() {
    assert_eq!(
        connect("wrong", None, 22).await.err(),
        Some(SshError::AuthenticationFailed)
    );
    assert!(matches!(
        connect("Tester-Passw0rd!", None, 2222).await,
        Err(SshError::Unreachable(_))
    ));
}
