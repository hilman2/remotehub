//! The site connector (ADR 0008, ADR 0011): runs in a network remotehub
//! cannot reach and carries TCP connections from there to remotehub, while
//! the customer keeps access open.
//!
//! - [`agent`]: the way to remotehub
//! - [`access`], [`journal`]: whether the customer lets remotehub in, and
//!   the customer's record of it
//! - [`ui`], [`https`], [`users`]: the web interface where the customer
//!   opens and closes access
//! - [`protocol`]: what the connector and remotehub say to each other; the
//!   server uses the same types
//! - [`network`]: address ranges, for `REMOTEHUB_CONNECTOR_ALLOW` here and
//!   the trusted proxies in the server

pub mod access;
pub mod agent;
pub mod files;
pub mod https;
pub mod journal;
pub mod network;
pub mod protocol;
pub mod settings;
pub mod ui;
pub mod users;

/// The release, from the workspace's `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
