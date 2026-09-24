//! Files generated from Rust for the UI stay in sync with the code. Run with
//! `REMOTEHUB_BLESS=1` to rewrite them after a change.

use std::path::PathBuf;

use remotehub_server::api::problem;
use remotehub_server::audit;

fn check(relative: &str, expected: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    if std::env::var_os("REMOTEHUB_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, expected).unwrap();
        return;
    }
    let actual = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        actual == expected,
        "{relative} is stale — regenerate with: REMOTEHUB_BLESS=1 cargo nextest run -p remotehub-server generated"
    );
}

#[test]
fn generated_error_codes_are_up_to_date() {
    check(
        "web/src/lib/api/generated/problem.ts",
        &problem::typescript(),
    );
}

#[test]
fn generated_audit_actions_are_up_to_date() {
    check("web/src/lib/api/generated/audit.ts", &audit::typescript());
}
