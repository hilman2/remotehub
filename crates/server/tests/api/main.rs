//! The server's integration tests, in one test binary: every file in
//! `tests/` would otherwise be linked into its own binary with the whole
//! server in it. nextest still runs every test in its own process, in
//! parallel. Tests marked `#[sqlx::test]` get a fresh database with all
//! migrations (needs `DATABASE_URL`, provided by the development compose and
//! the local CI).

mod common;

mod access;
mod accounts;
mod audit;
mod break_glass;
mod catalog;
mod certificate;
mod collections;
mod confirm;
mod connector_requests;
mod connectors;
mod directory;
mod display;
mod downloads;
mod extension;
mod generated;
mod generator;
mod groups;
mod http;
mod journal;
mod mail;
mod personal;
mod profiles;
mod recovery;
mod refresh;
mod reports;
mod requests;
mod roles;
mod search;
mod second_factor;
mod secrets;
mod session;
mod setup;
mod terminal;
mod users;
mod vault;
