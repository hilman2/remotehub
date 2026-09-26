# remotehub

**Browser-based remote access and credential vault for administrators.**

remotehub is a self-hosted web application in the spirit of classic RDP managers (mRemoteNG, Royal TS,
Remote Desktop Manager) combined with a multi-user password vault in the spirit of KeePass:

- Sign in with your **Active Directory** account (Entra ID follows).
- See the devices you are allowed to reach — RDP, VNC, SSH, HTTPS web interfaces — organised in folders.
- Click and work **in the browser**. The connection runs on the server and only picture, terminal output and
  input travel to your browser, so your client needs no network access to the targets.
- Reach other sites and segments through a **site connector** that connects out to remotehub, without
  inbound firewall rules. The customer opens and closes access at the connector, for a while or until
  further notice, and keeps a log of every connection.
- Connect with the **credentials stored for the device**, with **personal credentials**, or with **your own AD account** —
  stored passwords can be used without ever being shown (`connect` and `reveal` are separate permissions).
- Keep all other credentials in a **vault with folder permissions based on AD groups**, import existing
  **KeePass (KDBX)** databases and keep an encrypted offline copy for emergencies.

User interface in English and German from day one.

> **Status: early development.** Nothing here is ready for production yet. Until a second factor for
> directory sign-ins exists (milestone M4), remotehub is meant for internal networks only.

## How it works

```
Browser (SvelteKit SPA · xterm.js · Guacamole JS client)
   │ HTTPS + WebSocket only
remotehub (Rust: API, sign-in, permissions, vault, audit, SSH engine, Guacamole tunnel)
   ├── PostgreSQL
   ├── guacd (RDP and VNC engine, internal network only)
   └── browser service (Chromium for web interfaces, internal network only)
```

Everything runs in Docker. Details: [`docs/architecture.md`](docs/architecture.md), decisions in [`docs/adr/`](docs/adr/).
Installing: [`docs/install.md`](docs/install.md), settings: [`docs/configuration.md`](docs/configuration.md).

## Roadmap

| Milestone | Content |
|---|---|
| M0 Foundation | Workspace, local CI, development environment, i18n with enforcing tests |
| M1 First connection | AD sign-in via LDAP, folders and permissions, vault core, SSH in the browser, audit log |
| M2 RDP and VNC | guacd engine, session tabs, clipboard, first release 0.1.0 |
| M3 Vault | KeePass replacement: reveal with audit, history, personal vault, KDBX import and export |
| M4 Identity | Entra ID (OIDC), second factor (TOTP, WebAuthn) |
| M5 Accountability | Session recording and playback, audit export, clipboard and file policies |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).

## License

[GNU Affero General Public License v3.0](LICENSE). If you run a modified remotehub as a service for others,
you must offer them its source code.
