# ADR 0001: Architecture and stack

- Status: accepted
- Date: 2026-09-24

## Context

remotehub combines browser-based remote access (RDP, VNC, SSH, later HTTPS) with a multi-user credential
vault for administrators. It is open source, self-hosted, and meant to be run by one organisation per
instance. The product owner wants Rust at the core, everything in Docker, and a UI in English and German
from day one.

Existing products were checked before building (September 2026):

- **Apache Guacamole 1.6** — mature engine (guacd), but no vault, OIDC only via implicit flow (Entra passes
  at most five groups), dated UI.
- **Warpgate 0.29** (Rust, Apache-2.0) — SSH, RDP and VNC in the browser rendered on the server, OIDC, LDAP,
  recording, approvals. A bastion without a password vault, folder permissions by directory group, personal
  credentials or KeePass import.
- **Devolutions Server + Gateway** — the closest feature set (credential injection, vault), but proprietary.
- **JumpServer** — SSO and password rotation only in the enterprise edition.
- **Teleport** — no vault; the free edition is restricted by license.

Building on Warpgate (fork or contribution) was considered and rejected by the product owner: the vault,
the folder and group permission model and the AD-centric workflow are the heart of remotehub and would not
fit a bastion's architecture.

## Decision

- **Server:** one Rust binary (edition 2024) on tokio and axum. It serves the API, WebSockets and the static
  SPA. Crates by responsibility: `server`, `model`, `vault`, `directory`, `gateway`, `i18n` (later `kdbx`).
- **Database:** PostgreSQL via sqlx, forward-only migrations applied at startup.
- **Frontend:** SvelteKit 2 with Svelte 5, TypeScript and Tailwind 4 as a static SPA (`adapter-static`).
  Same stack as the maintainer's other Rust project, whose i18n guard tests carry over.
- **Protocols run on the server** (ADR 0003); guacd is the only non-Rust runtime component.
- **Everything runs in Docker**, in development as in production. One organisation per instance, no
  multi-tenancy.
- **License:** AGPL-3.0. All dependencies found so far (MIT, Apache-2.0, MPL-2.0) are compatible; guacd
  (links GPL-licensed libvncclient) stays a separate process.

## Consequences

- One binary plus PostgreSQL plus guacd is the whole deployment.
- Multi-tenancy (e.g. for service providers) would need a new decision and touches every permission check.
- Contributors need Rust and TypeScript; the development environment in Docker hides the toolchains.
