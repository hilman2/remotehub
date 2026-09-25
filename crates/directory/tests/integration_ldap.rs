//! Sign-in against the Samba domain of the test lab (deploy/testlab/dc,
//! users in users.sh). Needs `REMOTEHUB_TEST_LDAP_URL`; run with
//! `cargo nextest run --run-ignored only`.

use std::path::PathBuf;
use std::time::Duration;

use remotehub_directory::check::{Reason, Step, check};
use remotehub_directory::ldap::{LdapConfig, LdapDirectory};
use remotehub_directory::{AuthError, Identity, IdentityProvider, Sid};
use secrecy::SecretString;

fn lab_file(path: &str) -> PathBuf {
    // At run time: a test binary must not depend on the path it was built under.
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo and nextest"))
        .join("../../deploy/testlab")
        .join(path)
}

fn lab_text(path: &str) -> String {
    std::fs::read_to_string(lab_file(path)).expect("a file of the test lab")
}

fn config() -> LdapConfig {
    LdapConfig {
        url: std::env::var("REMOTEHUB_TEST_LDAP_URL")
            .expect("REMOTEHUB_TEST_LDAP_URL points to the test lab's domain controller"),
        starttls: false,
        ca_pem: Some(lab_text("dc/tls/ca.crt")),
        bind_dn: "svc-remotehub@remotehub.test".into(),
        bind_password: SecretString::from("Svc-Passw0rd!"),
        base_dn: "DC=remotehub,DC=test".into(),
        user_filter: None,
        timeout: Duration::from_secs(10),
    }
}

fn directory() -> LdapDirectory {
    LdapDirectory::new(config()).unwrap()
}

async fn sign_in(name: &str, password: &str) -> Result<Identity, AuthError> {
    directory()
        .authenticate(name, &SecretString::from(password))
        .await
}

async fn group_sid(name: &str) -> Sid {
    let groups = directory().search_groups(name, 10).await.unwrap();
    groups
        .into_iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("group {name} not found"))
        .sid
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn alice_signs_in_with_identity_and_groups() {
    let alice = sign_in("alice", "Alice-Passw0rd!").await.unwrap();
    assert_eq!(alice.username, "alice");
    assert_eq!(alice.display_name, "Alice Admin");
    assert_eq!(alice.upn.as_deref(), Some("alice@remotehub.test"));
    assert_eq!(alice.email.as_deref(), Some("alice@remotehub.test"));
    assert!(alice.sid.as_str().starts_with("S-1-5-21-"));
    assert!(alice.groups.contains(&group_sid("RH Admins").await));
    // Domain Users (RID 513) is the primary group and part of tokenGroups.
    assert!(alice.groups.iter().any(|g| g.rid() == Some(513)));
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn nested_groups_are_resolved() {
    let bob = sign_in("bob", "Bob-Passw0rd!").await.unwrap();
    assert!(bob.groups.contains(&group_sid("Helpdesk").await));
    assert!(bob.groups.contains(&group_sid("RH Operators").await));
    assert!(!bob.groups.contains(&group_sid("RH Admins").await));
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn every_name_form_finds_the_same_account() {
    let plain = sign_in("bob", "Bob-Passw0rd!").await.unwrap();
    for name in ["REMOTEHUB\\bob", "bob@remotehub.test", "BOB", " bob "] {
        let other = sign_in(name, "Bob-Passw0rd!").await.unwrap();
        assert_eq!(other.sid, plain.sid, "{name}");
        assert_eq!(other.guid, plain.guid, "{name}");
    }
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn wrong_unknown_empty_and_wildcard_are_invalid_credentials() {
    for (name, password) in [
        ("alice", "wrong"),
        ("nobody", "Alice-Passw0rd!"),
        ("alice", ""),
        ("*", "Alice-Passw0rd!"),
        ("al*", "Alice-Passw0rd!"),
    ] {
        assert_eq!(
            sign_in(name, password).await.unwrap_err(),
            AuthError::InvalidCredentials,
            "{name} / {password}"
        );
    }
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn disabled_accounts_are_reported() {
    assert_eq!(
        sign_in("carol", "Carol-Passw0rd!").await.unwrap_err(),
        AuthError::AccountDisabled
    );
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn a_refresh_reads_groups_and_state_without_a_password() {
    let bob = sign_in("bob", "Bob-Passw0rd!").await.unwrap();
    assert_eq!(directory().refresh(&bob.sid).await.unwrap(), bob.groups);

    let carol = directory()
        .search_users("carol", 5)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.detail.as_deref().is_some_and(|d| d.starts_with("carol")))
        .expect("carol is in the lab")
        .sid;
    assert_eq!(
        directory().refresh(&carol).await.unwrap_err(),
        AuthError::AccountDisabled
    );

    let nobody: Sid = "S-1-5-21-1-2-3-999999".parse().unwrap();
    assert_eq!(
        directory().refresh(&nobody).await.unwrap_err(),
        AuthError::InvalidCredentials
    );
    // Outside the user filter counts as gone.
    let only_admins = LdapDirectory::new(LdapConfig {
        user_filter: Some(
            "(memberOf:1.2.840.113556.1.4.1941:=CN=RH Admins,CN=Users,DC=remotehub,DC=test)".into(),
        ),
        ..config()
    })
    .unwrap();
    assert_eq!(
        only_admins.refresh(&bob.sid).await.unwrap_err(),
        AuthError::InvalidCredentials
    );
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn users_without_groups_only_have_builtin_ones() {
    let dave = sign_in("dave", "Dave-Passw0rd!").await.unwrap();
    for group in ["RH Admins", "RH Operators", "Helpdesk"] {
        assert!(!dave.groups.contains(&group_sid(group).await), "{group}");
    }
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn an_extra_user_filter_restricts_sign_in() {
    let admins = group_sid("RH Admins").await;
    let only_admins = LdapDirectory::new(LdapConfig {
        user_filter: Some(
            "(memberOf:1.2.840.113556.1.4.1941:=CN=RH Admins,CN=Users,DC=remotehub,DC=test)".into(),
        ),
        ..config()
    })
    .unwrap();
    let alice = only_admins
        .authenticate("alice", &SecretString::from("Alice-Passw0rd!"))
        .await
        .unwrap();
    assert!(alice.groups.contains(&admins));
    assert_eq!(
        only_admins
            .authenticate("bob", &SecretString::from("Bob-Passw0rd!"))
            .await
            .unwrap_err(),
        AuthError::InvalidCredentials
    );
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn an_untrusted_certificate_makes_the_directory_unavailable() {
    // Another server's certificate: neither a CA nor the DC's own.
    let untrusting = LdapDirectory::new(LdapConfig {
        ca_pem: Some(lab_text("desktop/tls/cert.pem")),
        ..config()
    })
    .unwrap();
    let error = untrusting
        .authenticate("alice", &SecretString::from("Alice-Passw0rd!"))
        .await
        .unwrap_err();
    assert!(matches!(error, AuthError::Unavailable(_)), "{error:?}");
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn the_domain_controllers_own_certificate_can_be_trusted_as_it_is() {
    let pinned = LdapDirectory::new(LdapConfig {
        ca_pem: Some(lab_text("dc/tls/dc.crt")),
        ..config()
    })
    .unwrap();
    let alice = pinned
        .authenticate("alice", &SecretString::from("Alice-Passw0rd!"))
        .await
        .unwrap();
    assert_eq!(alice.username, "alice");
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn a_check_names_the_step_that_fails() {
    let failed = |config: LdapConfig| async move {
        let failure = check(config).await.unwrap_err();
        (failure.step, failure.reason)
    };
    let url = config().url;
    let host = url.trim_start_matches("ldaps://").to_owned();
    assert_eq!(
        failed(LdapConfig {
            url: "ldaps://nowhere.remotehub.test".into(),
            ..config()
        })
        .await,
        (Step::Resolve, Reason::NotFound)
    );
    assert_eq!(
        failed(LdapConfig {
            url: format!("ldaps://{host}:1"),
            ..config()
        })
        .await,
        (Step::Connect, Reason::Refused)
    );
    assert_eq!(
        failed(LdapConfig {
            bind_password: SecretString::from("wrong"),
            ..config()
        })
        .await,
        (Step::Bind, Reason::InvalidCredentials)
    );
    assert_eq!(
        failed(LdapConfig {
            base_dn: "OU=nowhere,DC=remotehub,DC=test".into(),
            ..config()
        })
        .await,
        (Step::Search, Reason::NoSuchBase)
    );
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn an_unknown_ca_comes_back_with_the_certificate_to_trust() {
    // The system's roots do not know the lab's CA, and the DC presents only
    // its own certificate.
    let failure = check(LdapConfig {
        ca_pem: None,
        ..config()
    })
    .await
    .unwrap_err();
    assert_eq!(
        (failure.step, failure.reason),
        (Step::Tls, Reason::UnknownCa)
    );
    let presented = failure.ca.expect("the certificate to trust");
    assert!(!presented.authority);
    assert_eq!(presented.subject, "CN=dc.remotehub.test");
    assert_eq!(presented.fingerprint.len(), 32 * 3 - 1);

    // Trusted as it is, the check passes and finds the lab's users.
    let found = check(LdapConfig {
        ca_pem: Some(presented.pem),
        ..config()
    })
    .await
    .unwrap();
    assert!(found.users >= 5, "{found:?}");
    assert!(!found.more);
    assert_eq!(found.sample.len(), 3);
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn a_check_passes_with_start_tls_too() {
    let url = config().url;
    let host = url.trim_start_matches("ldaps://").to_owned();
    let found = check(LdapConfig {
        url: format!("ldap://{host}"),
        starttls: true,
        ..config()
    })
    .await
    .unwrap();
    assert!(found.users >= 5, "{found:?}");
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn a_wrong_service_password_is_a_directory_error() {
    let misconfigured = LdapDirectory::new(LdapConfig {
        bind_password: SecretString::from("wrong"),
        ..config()
    })
    .unwrap();
    let error = misconfigured
        .authenticate("alice", &SecretString::from("Alice-Passw0rd!"))
        .await
        .unwrap_err();
    assert!(matches!(error, AuthError::Directory(_)), "{error:?}");
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn groups_can_be_searched_by_part_of_their_name() {
    let names: Vec<String> = directory()
        .search_groups("RH ", 10)
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.name)
        .collect();
    assert_eq!(names, ["RH Admins", "RH Operators"]);
}

#[tokio::test]
#[ignore = "needs the test lab"]
async fn laps_passwords_are_found_by_host_name() {
    use remotehub_directory::laps::LapsError;
    use secrecy::ExposeSecret;

    let directory = directory();
    for host in [
        "desktop-target",
        "desktop-target.remotehub.test",
        "DESKTOP-TARGET.",
    ] {
        let laps = directory.laps_password(host).await.unwrap();
        assert_eq!(laps.account, "tester", "{host}");
        assert_eq!(laps.password.expose_secret(), "Tester-Passw0rd!");
        assert_eq!(laps.computer, "desktop-target");
    }
    assert!(matches!(
        directory.laps_password("nolaps").await,
        Err(LapsError::NoPassword(_))
    ));
    for host in ["nowhere", "10.0.0.5", "*"] {
        assert!(
            matches!(
                directory.laps_password(host).await,
                Err(LapsError::ComputerNotFound(_))
            ),
            "{host}"
        );
    }
}
