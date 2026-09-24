//! Protocol engines: connections run on the server, the browser only sees
//! their output (ADR 0003). SSH runs here with russh; RDP and VNC run in
//! guacd, which this crate speaks to.

pub mod guacamole;
pub mod rdp;
pub mod ssh;
pub mod ssh_ca;
