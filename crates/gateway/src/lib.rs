//! Protocol engines: connections run on the server, the browser only sees
//! their output (ADR 0003). SSH runs here with russh; RDP and VNC follow via
//! guacd (M2).

pub mod ssh;
