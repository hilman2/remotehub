//! The site connector (ADR 0008): runs in a network remotehub cannot reach
//! and carries TCP connections from there to remotehub.
//!
//! - [`agent`]: the connector itself, `remotehub-connector`
//! - [`protocol`]: what it and remotehub say to each other; the server uses
//!   the same types
//! - [`network`]: address ranges, for `REMOTEHUB_CONNECTOR_ALLOW` here and
//!   the trusted proxies in the server

pub mod agent;
pub mod network;
pub mod protocol;

/// The release, from the workspace's `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
