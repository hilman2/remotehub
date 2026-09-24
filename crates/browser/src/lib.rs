//! Web interfaces of devices in a browser on the server (ADR 0007).
//!
//! The agent (`remotehub-browser`, [`session`]) runs in the browser
//! container: for every connection from remotehub it starts an Xvnc display
//! and a Chromium that reaches only the device ([`proxy`]), signs in through
//! the DevTools Protocol ([`cdp`], [`fill`]) and ends both when the
//! connection closes. remotehub talks to it through [`client`]; the wire
//! format is in [`protocol`].

pub mod cdp;
pub mod client;
pub mod fill;
pub mod protocol;
pub mod proxy;
pub mod session;
