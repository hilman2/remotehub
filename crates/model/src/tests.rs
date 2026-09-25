//! Table-driven tests for `authorize()`.
//!
//! Tree and grants:
//!
//! ```text
//! Servers            ops: connect
//! ├── Linux          linux: edit
//! │   ├── web01      (device)
//! │   └── root-pw    (credential)   alice: reveal
//! └── Windows
//!     ├── dc01       (device)       helpdesk: list
//!     └── da-pw      (credential)
//! Network            net: manage
//! └── switch01       (device)
//! ```

use super::*;

const SERVERS: Uuid = Uuid::from_u128(1);
const LINUX: Uuid = Uuid::from_u128(2);
const WINDOWS: Uuid = Uuid::from_u128(3);
const NETWORK: Uuid = Uuid::from_u128(4);
const WEB01: Uuid = Uuid::from_u128(11);
const DC01: Uuid = Uuid::from_u128(12);
const SWITCH01: Uuid = Uuid::from_u128(13);
const ROOT_PW: Uuid = Uuid::from_u128(21);
const DA_PW: Uuid = Uuid::from_u128(22);

/// The catalogs' "now", in Unix seconds.
const NOW: i64 = 1_800_000_000;

fn grant(object: ObjectId, principal: &str, role: Role) -> Grant {
    Grant {
        object,
        principal: principal.to_owned(),
        role,
        until: None,
    }
}

fn catalog() -> Catalog {
    Catalog::new(
        [
            (SERVERS, None),
            (LINUX, Some(SERVERS)),
            (WINDOWS, Some(SERVERS)),
            (NETWORK, None),
        ],
        [(WEB01, LINUX), (DC01, WINDOWS), (SWITCH01, NETWORK)],
        [(ROOT_PW, LINUX), (DA_PW, WINDOWS)],
        [
            grant(ObjectId::Folder(SERVERS), "S-ops", Role::Connect),
            grant(ObjectId::Folder(LINUX), "S-linux", Role::Edit),
            grant(ObjectId::Device(DC01), "S-helpdesk", Role::List),
            grant(ObjectId::Credential(ROOT_PW), "S-alice", Role::Reveal),
            grant(ObjectId::Folder(NETWORK), "S-net", Role::Manage),
        ],
        NOW,
    )
}

fn subject(sids: &[&str]) -> Subject {
    Subject {
        sids: sids.iter().map(|s| (*s).to_owned()).collect(),
        admin: false,
    }
}

fn admin() -> Subject {
    Subject {
        sids: HashSet::new(),
        admin: true,
    }
}

use ObjectId::{Credential, Device, Folder};

#[test]
fn effective_roles() {
    let catalog = catalog();
    let ops = subject(&["S-user-1", "S-ops"]);
    let linux_ops = subject(&["S-user-2", "S-ops", "S-linux"]);
    let helpdesk = subject(&["S-user-3", "S-helpdesk"]);
    let alice = subject(&["S-alice"]);
    let net = subject(&["S-net"]);
    let nobody = subject(&["S-nobody"]);
    let admin = admin();

    #[rustfmt::skip]
    let table: &[(&str, &Subject, ObjectId, Option<Role>)] = &[
        // A grant on a folder holds for everything below it.
        ("ops on Servers",        &ops, Folder(SERVERS),  Some(Role::Connect)),
        ("ops on Linux",          &ops, Folder(LINUX),    Some(Role::Connect)),
        ("ops on web01",          &ops, Device(WEB01),    Some(Role::Connect)),
        ("ops on dc01",           &ops, Device(DC01),     Some(Role::Connect)),
        ("ops on da-pw",          &ops, Credential(DA_PW), Some(Role::Connect)),
        ("ops outside",           &ops, Device(SWITCH01), None),
        // The highest role of all of a person's SIDs counts.
        ("linux+ops on web01",    &linux_ops, Device(WEB01),     Some(Role::Edit)),
        ("linux+ops on root-pw",  &linux_ops, Credential(ROOT_PW), Some(Role::Edit)),
        ("linux+ops on dc01",     &linux_ops, Device(DC01),      Some(Role::Connect)),
        ("linux+ops on Servers",  &linux_ops, Folder(SERVERS),   Some(Role::Connect)),
        // A grant on one object says nothing about its neighbours or parents.
        ("helpdesk on dc01",      &helpdesk, Device(DC01),       Some(Role::List)),
        ("helpdesk on da-pw",     &helpdesk, Credential(DA_PW),  None),
        ("helpdesk on Windows",   &helpdesk, Folder(WINDOWS),    None),
        ("alice on root-pw",      &alice, Credential(ROOT_PW),   Some(Role::Reveal)),
        ("alice on web01",        &alice, Device(WEB01),         None),
        ("net on switch01",       &net, Device(SWITCH01),        Some(Role::Manage)),
        ("net on Servers",        &net, Folder(SERVERS),         None),
        ("nobody on anything",    &nobody, Folder(SERVERS),      None),
        // Administrators manage everything that exists — and nothing else.
        ("admin on da-pw",        &admin, Credential(DA_PW),     Some(Role::Manage)),
        ("admin on unknown",      &admin, Device(Uuid::from_u128(99)), None),
        ("ops on unknown",        &ops, Folder(Uuid::from_u128(98)),   None),
    ];

    for (case, subject, object, expected) in table {
        assert_eq!(
            catalog.effective_role(subject, *object),
            *expected,
            "{case}"
        );
        // authorize() agrees with the effective role for every threshold.
        for needed in Role::ALL {
            assert_eq!(
                catalog.authorize(subject, needed, *object),
                expected.is_some_and(|role| role >= needed),
                "{case}: {needed:?}"
            );
        }
    }
}

#[test]
fn roles_are_ordered_and_named() {
    assert!(Role::List < Role::Connect);
    assert!(Role::Connect < Role::Reveal);
    assert!(Role::Reveal < Role::Edit);
    assert!(Role::Edit < Role::Manage);
    for role in Role::ALL {
        assert_eq!(Role::parse(role.as_str()), Some(role));
        assert_eq!(serde_json::to_value(role).unwrap(), role.as_str());
    }
    assert_eq!(Role::parse("owner"), None);
}

#[test]
fn only_administrators_create_at_the_top() {
    let catalog = catalog();
    assert!(catalog.may_create_top_level(&admin()));
    assert!(!catalog.may_create_top_level(&subject(&["S-net"])));
}

#[test]
fn visibility_includes_the_way_to_what_is_granted() {
    let catalog = catalog();
    let helpdesk = catalog.visible(&subject(&["S-helpdesk"]));
    assert_eq!(helpdesk.roles, HashMap::from([(Device(DC01), Role::List)]));
    assert_eq!(helpdesk.path_only, HashSet::from([SERVERS, WINDOWS]));

    let alice = catalog.visible(&subject(&["S-alice"]));
    assert_eq!(
        alice.roles.keys().collect::<Vec<_>>(),
        [&Credential(ROOT_PW)]
    );
    assert_eq!(alice.path_only, HashSet::from([SERVERS, LINUX]));

    let ops = catalog.visible(&subject(&["S-ops"]));
    assert_eq!(
        ops.roles.len(),
        7,
        "Servers, Linux, Windows and their four entries"
    );
    assert!(ops.path_only.is_empty());

    assert_eq!(
        catalog.visible(&subject(&["S-nobody"])),
        Visibility::default()
    );
    assert_eq!(catalog.visible(&admin()).roles.len(), 9);
}

#[test]
fn containment() {
    let catalog = catalog();
    assert!(catalog.is_within(LINUX, SERVERS));
    assert!(catalog.is_within(SERVERS, SERVERS));
    assert!(!catalog.is_within(SERVERS, LINUX));
    assert!(!catalog.is_within(NETWORK, SERVERS));
    assert_eq!(catalog.parent(Device(WEB01)), Some(LINUX));
    assert_eq!(catalog.parent(Folder(SERVERS)), None);
}

#[test]
fn a_folder_loop_does_not_hang() {
    let a = Uuid::from_u128(1);
    let b = Uuid::from_u128(2);
    let catalog = Catalog::new(
        [(a, Some(b)), (b, Some(a))],
        [],
        [],
        [grant(Folder(a), "S-x", Role::Edit)],
        NOW,
    );
    assert_eq!(
        catalog.effective_role(&subject(&["S-x"]), Folder(b)),
        Some(Role::Edit)
    );
}

#[test]
fn a_just_in_time_grant_counts_until_it_runs_out() {
    let at = |now: i64| {
        let jit = Grant {
            until: Some(NOW + 3600),
            ..grant(Device(DC01), "S-helpdesk", Role::Connect)
        };
        let permanent = grant(Device(DC01), "S-helpdesk", Role::List);
        Catalog::new(
            [(WINDOWS, None)],
            [(DC01, WINDOWS)],
            [],
            [permanent, jit],
            now,
        )
    };
    let helpdesk = subject(&["S-helpdesk"]);
    assert_eq!(
        at(NOW).effective_role(&helpdesk, Device(DC01)),
        Some(Role::Connect)
    );
    assert_eq!(
        at(NOW + 3599).effective_role(&helpdesk, Device(DC01)),
        Some(Role::Connect)
    );
    // At the end and after it, only the permanent grant is left.
    for later in [NOW + 3600, NOW + 86_400] {
        assert_eq!(
            at(later).effective_role(&helpdesk, Device(DC01)),
            Some(Role::List),
            "{later}"
        );
    }
}

#[test]
fn requests_go_up_to_reveal_and_approvals_need_manage() {
    let catalog = catalog();
    let helpdesk = subject(&["S-helpdesk"]);
    // helpdesk sees dc01 (list) and may ask for more.
    assert!(catalog.may_request(&helpdesk, Role::Connect, Device(DC01)));
    assert!(catalog.may_request(&helpdesk, Role::Reveal, Device(DC01)));
    // Not for changing things, not for what they already hold, not for what
    // they cannot see.
    assert!(!catalog.may_request(&helpdesk, Role::Edit, Device(DC01)));
    assert!(!catalog.may_request(&helpdesk, Role::List, Device(DC01)));
    assert!(!catalog.may_request(&helpdesk, Role::Connect, Device(WEB01)));
    let ops = subject(&["S-ops"]);
    assert!(!catalog.may_request(&ops, Role::Connect, Device(WEB01)));
    assert!(catalog.may_request(&ops, Role::Reveal, Device(WEB01)));

    assert!(catalog.may_approve(&subject(&["S-net"]), Device(SWITCH01)));
    assert!(catalog.may_approve(&admin(), Device(DC01)));
    assert!(!catalog.may_approve(&subject(&["S-linux"]), Device(WEB01)));
    assert!(!catalog.may_approve(&subject(&["S-net"]), Device(DC01)));
}

#[test]
fn the_grants_behind_a_role_are_named_nearest_first() {
    use ObjectId::*;
    let catalog = catalog();
    assert_eq!(
        catalog.grants_along(Credential(ROOT_PW)),
        [
            (Credential(ROOT_PW), "S-alice", Role::Reveal),
            (Folder(LINUX), "S-linux", Role::Edit),
            (Folder(SERVERS), "S-ops", Role::Connect),
        ]
    );
    // alice in ops: her own grant and the inherited one, not linux's.
    let alice = subject(&["S-alice", "S-ops"]);
    assert_eq!(
        catalog.reasons(&alice, Credential(ROOT_PW)),
        [
            (Credential(ROOT_PW), "S-alice", Role::Reveal),
            (Folder(SERVERS), "S-ops", Role::Connect),
        ]
    );
    // The highest reason is the effective role.
    assert_eq!(
        catalog.effective_role(&alice, Credential(ROOT_PW)),
        Some(Role::Reveal)
    );
    assert_eq!(catalog.reasons(&alice, Device(SWITCH01)), []);
}
