use time::macros::datetime;

use super::*;
use crate::inventory::Ports;

fn device(name: &str, groups: &[&str]) -> Device {
    Device {
        name: name.into(),
        address: "10.0.0.1".parse().unwrap(),
        ports: Ports::parse_list("22").unwrap(),
        groups: groups.iter().map(|g| (*g).to_owned()).collect(),
    }
}

fn open_names(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .open_devices(OffsetDateTime::now_utc())
        .iter()
        .map(|d| d.name.clone())
        .collect()
}

#[test]
fn open_means_before_its_end() {
    let now = datetime!(2026-10-01 12:00 UTC);
    assert!(!Access::Closed.is_open(now));
    assert!(Access::Open { until: None }.is_open(now));
    let until = Access::Open {
        until: Some(datetime!(2026-10-01 13:00 UTC)),
    };
    assert!(until.is_open(now));
    assert!(!until.is_open(datetime!(2026-10-01 13:00 UTC)));
}

#[test]
fn nothing_stored_is_closed_and_a_change_is_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::new(dir.path());
    assert_eq!(load(dir.path()).unwrap(), Snapshot::default());
    let until = Some(datetime!(2026-10-01 13:00 UTC));
    change(
        dir.path(),
        &journal,
        Scope::Network,
        Access::Open { until },
        Changer::CommandLine,
    )
    .unwrap();
    let stored = load(dir.path()).unwrap().access.network;
    assert_eq!(stored.access, Access::Open { until });
    assert_eq!(stored.changed_by, Some(Changer::CommandLine));
    assert_eq!(
        journal.recent(1)[0].event,
        Event::Opened {
            by: Changer::CommandLine,
            until,
            scope: Scope::Network
        }
    );
}

/// A device opens itself or through any of its groups; removing a group
/// takes back what it opened (#180).
#[test]
fn devices_open_themselves_or_through_their_groups() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::new(dir.path());
    let web = || Changer::Web {
        user: "anna".into(),
    };
    add_group(dir.path(), &journal, "ERP", web()).unwrap();
    add_device(dir.path(), &journal, device("sql", &["ERP"]), web()).unwrap();
    add_device(dir.path(), &journal, device("haproxy", &["ERP"]), web()).unwrap();
    add_device(dir.path(), &journal, device("files", &[]), web()).unwrap();
    let open = Access::Open { until: None };

    change(
        dir.path(),
        &journal,
        Scope::Group { name: "ERP".into() },
        open,
        web(),
    )
    .unwrap();
    let snapshot = load(dir.path()).unwrap();
    assert!(!snapshot.network_open(OffsetDateTime::now_utc()));
    assert_eq!(open_names(&snapshot), ["haproxy", "sql"]);

    change(
        dir.path(),
        &journal,
        Scope::Device {
            name: "files".into(),
        },
        open,
        web(),
    )
    .unwrap();
    remove_group(dir.path(), &journal, "ERP", web()).unwrap();
    let snapshot = load(dir.path()).unwrap();
    assert_eq!(open_names(&snapshot), ["files"]);
    assert!(snapshot.access.groups.is_empty());

    // What is not on the list cannot be opened.
    let unknown = change(
        dir.path(),
        &journal,
        Scope::Device {
            name: "nowhere".into(),
        },
        open,
        web(),
    );
    assert!(matches!(
        unknown,
        Err(ChangeError::Inventory(InventoryError::Unknown(_)))
    ));
    remove_device(dir.path(), &journal, "files", web()).unwrap();
    assert!(
        !load(dir.path())
            .unwrap()
            .any_open(OffsetDateTime::now_utc())
    );

    let kinds: Vec<&str> = journal
        .recent(20)
        .iter()
        .map(|entry| match entry.event {
            Event::DeviceAdded { .. } => "device added",
            Event::DeviceRemoved { .. } => "device removed",
            Event::GroupAdded { .. } => "group added",
            Event::GroupRemoved { .. } => "group removed",
            Event::Opened { .. } => "opened",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "device removed",
            "group removed",
            "opened",
            "opened",
            "device added",
            "device added",
            "device added",
            "group added"
        ]
    );
}

/// The running connector takes over a change the command line wrote, and
/// closes a device itself once its time ran out.
#[tokio::test]
async fn the_gate_follows_the_files_and_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Arc::new(Journal::new(dir.path()));
    add_device(
        dir.path(),
        &journal,
        device("sql", &[]),
        Changer::CommandLine,
    )
    .unwrap();
    let gate = Gate::new(dir.path(), journal.clone()).unwrap();
    let mut seen = gate.subscribe();
    let _follow = tokio::spawn(gate.clone().follow());

    let sql = Scope::Device { name: "sql".into() };
    let until = OffsetDateTime::now_utc() + Duration::from_secs(2);
    change(
        dir.path(),
        &journal,
        sql.clone(),
        Access::Open { until: Some(until) },
        Changer::CommandLine,
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), seen.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(open_names(&seen.borrow_and_update()), ["sql"]);

    tokio::time::timeout(Duration::from_secs(5), seen.changed())
        .await
        .unwrap()
        .unwrap();
    let closed = seen.borrow().access.get(&sql);
    assert_eq!(closed.access, Access::Closed);
    assert_eq!(closed.changed_by, Some(Changer::Expiry));
    assert_eq!(journal.recent(1)[0].event, Event::Expired { scope: sql });
}
