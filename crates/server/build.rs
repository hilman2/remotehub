// `sqlx::migrate!` embeds migrations/ at compile time; without this, cargo
// would not rebuild the server when only a migration changes (incremental
// builds in development and in the local CI).
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
