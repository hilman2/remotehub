//! The server's integration tests, in one test binary: every file in
//! `tests/` would otherwise be linked into its own binary with the whole
//! server in it. nextest still runs every test in its own process, in
//! parallel. Tests marked `#[sqlx::test]` get a fresh database with all
//! migrations (needs `DATABASE_URL`, provided by the development compose and
//! the local CI).

mod common;

mod audit;
mod break_glass;
mod catalog;
mod generated;
mod http;
mod secrets;
mod session;
mod terminal;
