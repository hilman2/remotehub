# ADR 0003: Protocols run on the server

- Status: accepted
- Date: 2026-09-24

## Context

Connections could run in the browser (an RDP or VNC client compiled to WebAssembly or JavaScript, reaching
the target through a WebSocket relay) or on the server (the server is the client and streams only picture
and terminal output).

Browser-side clients need the credentials in the browser, or a gateway that intercepts the authentication
(e.g. a CredSSP man-in-the-middle for RDP). Recording would happen in the client and could be tampered with.
The IronRDP web client also lacks audio and drive redirection today, and the VNC and SSH web clients of
Devolutions are not publicly available.

The product owner prefers server-side connections: the client then needs no network access to the targets,
and credentials never leave the server.

## Decision

- **Every protocol runs on the server.** The browser displays and sends input, nothing more.
- **SSH:** russh inside `crates/gateway`. The WebSocket carries terminal bytes and resize events; xterm.js
  renders them. This gives a real terminal (selection, search, scrollback) and small asciicast recordings.
- **RDP and VNC:** guacd 1.6 (Apache Guacamole's proxy daemon) in its own container, built with FreeRDP 3.
  remotehub implements the Guacamole protocol client side in Rust (no maintained crate exists): it performs
  the handshake, passes the credentials in `connect`, then relays instructions to the browser, where the
  Guacamole JS client (vendored from guacamole-client 1.6.0, not the outdated npm copy) draws them.
- **HTTPS:** a Chromium per session, shown through guacd; logins filled in via the Chrome DevTools
  Protocol. How: ADR 0007.
- **Engine abstraction:** all engines implement the trait `ProtocolEngine`. The API and UI do not know
  which engine serves a protocol.
- **Hardening of guacd:** no published ports, reachable only by the server on an internal network, non-root,
  read-only file system, resource limits, pinned version.
- **Target identity:** SSH host keys and RDP certificates are pinned on first use; a change aborts the
  connection until an admin confirms it (audited).

## Consequences

- guacd is C code with a history of CVEs and is CPU-bound (rule of thumb: 1 vCPU and 2 GB RAM per 25
  sessions); larger installations need several guacd containers with sticky routing.
- guacd 1.6 authenticates RDP with NTLM only. Domains without NTLM and members of *Protected Users* are not
  supported until guacd 1.7 brings Kerberos (GUACAMOLE-2057).
- guacd links GPL-licensed libvncclient; it stays a separate process and container, nothing of it is linked
  into the AGPL Rust server.
- Recording (M5) happens on the server and cannot be forged by the client.
- An own RDP engine (IronRDP on the server, streaming Guacamole `img` instructions) can replace guacd later
  behind the same trait; it is a backlog item, estimated at several person-months. Evaluated in ADR 0006.
