# Architecture of remotehub

## Context

Administrators work with dozens to thousands of systems over RDP, VNC and SSH, plus web interfaces of
appliances. Classic tools are desktop applications (mRemoteNG, Royal TS, Remote Desktop Manager) with
credentials in a KeePass file or a proprietary server. remotehub moves both into one self-hosted web
application:

- Sign in with the directory account (Active Directory first, Entra ID later).
- See the devices you may reach, organised in folders, and connect **in the browser**.
- Use stored credentials **without seeing them**, personal credentials, or your own AD account.
- Keep all other credentials in a multi-user vault with permissions based on directory groups.

Decisions and their reasons are in [`docs/adr/`](adr/). This document describes the whole; it is updated in
the same pull request as any change to it.

## 1. Components

```
Browser: SvelteKit SPA (devices, vault, session tabs) · xterm.js · Guacamole JS client
   │  HTTPS + WebSocket only — no route to the targets needed
remotehub server (Rust, one binary: axum on tokio)
   ├─ api        REST + WebSocket, RFC 9457 problems with ErrorCode, serves the SPA
   ├─ directory  IdentityProvider: LDAP/AD (M1), OIDC/Entra ID (M4)
   ├─ model      domain types and authorize() — the only place for permissions
   ├─ vault      envelope encryption, KeyProvider (key file first)
   ├─ gateway    ProtocolEngine: SSH (russh), RDP/VNC through the Guacamole tunnel
   ├─ i18n       Message + Fluent catalogs for server-rendered text
   └─ audit      append-only, hash-chained event log
        │                                 │ internal Docker network, no published ports
PostgreSQL (sqlx, migrations at start)    guacd 1.6 (own image with FreeRDP 3)
                                          browser service (Chromium on Xvnc, one per HTTPS session)

Other networks: site connector (remotehub connector) ──WebSocket, outbound──▶ remotehub
```

| Crate | Role |
|---|---|
| `crates/server` | Binary: configuration, HTTP/WebSocket API, sessions, static SPA |
| `crates/model` | Domain types (folders, devices, credentials, grants) and `authorize()` |
| `crates/vault` | Encryption of secrets, `KeyProvider`, credential storage |
| `crates/directory` | `IdentityProvider` with the LDAP implementation |
| `crates/gateway` | `ProtocolEngine`: SSH engine, Guacamole tunnel to guacd, certificates of devices |
| `crates/browser` | The browser service's agent (`remotehub-browser`) and remotehub's client for it |
| `crates/i18n` | `Message`, Fluent catalogs, locale negotiation |
| `crates/kdbx` (M3) | KeePass import and emergency export, isolated because its dependency moves fast |

Crates are created with the issue that first needs them.

## 2. A connection, step by step

1. The user clicks a device. The UI opens a WebSocket to `/api/sessions/{device}`.
2. The server checks the session cookie and calls `authorize(user, Connect, device)`.
3. It resolves the credentials according to the device's mode:
   - **stored** — decrypted from the vault on the server (`connect` suffices, `reveal` is not needed);
   - **ask** — entered by the user for this connection, never stored;
   - **own AD account** — the sign-in password, kept encrypted with a key that only the user's cookie holds.
4. The server starts the engine:
   - **SSH:** russh connects, verifies the pinned host key, authenticates, opens a PTY. Only terminal
     bytes and resize events cross the WebSocket; xterm.js renders them. A stored credential is a
     password or an SSH key (OpenSSH or PEM, optionally with passphrase and OpenSSH user certificate); a
     key is parsed and matched to its certificate before it is sealed, and shown only by fingerprint.
   - **RDP/VNC:** the server opens a TCP connection to guacd, performs the Guacamole handshake
     (`select → args → connect → ready`) and puts the credentials into `connect`. From then on it relays
     Guacamole instructions between guacd and the browser, where the Guacamole JS client draws them.
5. Start, end and outcome of the session are written to the audit log.

The browser never talks to a target or to guacd, and never receives a stored password.

## 3. Identity and permissions

- **Sign-in (M1):** bind to AD via LDAPS as the user to verify the password; read attributes and all
  nested groups through a service account from `tokenGroups` (fallback `LDAP_MATCHING_RULE_IN_CHAIN`).
- **Identifiers:** users and groups are stored by `objectSid`/`objectGUID` (Entra object IDs later), never
  by name, so renames do not change permissions.
- **Sessions:** server-side in PostgreSQL; cookie HttpOnly, Secure, SameSite=Strict; CSRF protection.
- **Break-glass:** local accounts (argon2id + TOTP, each code accepted once), created, reset and deleted
  only via `remotehub break-glass …`, which prints password and TOTP secret once. The TOTP secret is sealed
  in the vault. They sign in at `/sign-in/break-glass`, work when AD is down, are administrators, and every
  attempt is audited with `break_glass: true`; the UI shows a red banner during such a session.
- **Permissions:** a folder tree holds devices and credentials. A grant gives an AD group or a user a role
  on a folder or an entry: `list < connect < reveal < edit < manage`. Grants are inherited downwards and
  only allow. `authorize()` is the single decision point and is tested table-driven.
- **Just-in-time access:** someone who sees an object asks for `connect` or `reveal` on it for up to a day,
  with a reason (`/api/access-requests`). Someone else who manages the object approves or denies; nobody
  decides their own request. An approval becomes a user grant with `expires_at`, and the catalog, built
  with the database's `now`, drops it once it has run out; open sessions keep running. Every step is
  audited (`access.*`).
- **Stored credentials only go where their users may send them:** linking a credential to a device, or
  changing protocol, host or port of a device that has one, needs `connect` on that credential. Otherwise
  anyone with `edit` on a device could point it at their own server and capture the password. A device's
  own credentials (sign-in mode `device`, password sealed with the device as owner) follow the same rule:
  another target, connector included, keeps them only if the password is entered again.
- Objects a user cannot see answer `not_found`, so their existence does not leak; visible objects the user
  may not change answer `forbidden`. Folders on the way to something visible are shown without a role.

## 4. Vault

- Own data model; KeePass files are only imported and written as emergency exports (ADR 0004).
- Each entry version is encrypted with its own data key (XChaCha20-Poly1305). Associated data binds scheme,
  entry, field and key version, so ciphertexts cannot be swapped between rows.
- Data keys are wrapped by a versioned master key from a `KeyProvider` (first: a key file mounted as a Docker
  secret; later Vault/OpenBao Transit, Azure Key Vault, PKCS#11). Rows record `scheme`, `kek_id` and
  `kek_version` so keys can be rotated lazily.
- The key file holds one line per version (`<version>:<base64 key>`, made by `remotehub generate-key`);
  the highest version is current. Rotation: append a line, restart, rewrap old values.
- Sealed fields live in `secret_fields`, one row per owner, version and field; the associated data makes a
  row that is copied or moved elsewhere impossible to open.
- The server must be able to decrypt stored credentials to inject them. That rules out zero knowledge for
  shared entries.
- **Personal vault** (`e2e_user_v1`, `/vault`): entries only their owner can read, encrypted in the browser
  with WebCrypto (`web/src/lib/vault/crypto.ts`). One random vault key (AES-256-GCM, entry ID as associated
  data) is stored wrapped (AES-KW) once per way to unlock: a passkey's WebAuthn PRF output or the recovery
  key through HKDF, the passphrase through PBKDF2-SHA-256 (600 000 iterations). The server keeps
  ciphertext and wrapped keys for their owner only (`api/personal.rs`); nobody, the operator included, can
  reset the passphrase. The server cannot inject such entries into connections; the browser can: while
  the vault is unlocked (the key stays in memory until it is locked, the user signs out or the page
  reloads), a device that asks for credentials offers its entries, and the chosen one is sent as if
  typed.
- **Search** (`web/src/lib/search/rank.ts`) runs in the browser: parts of words in names, hosts,
  descriptions and paths, ranked by match and by what the user picked for the same or a similar query
  before. Picks from the device list are stored per user on the server (`search_picks`); picks in the
  personal vault are sealed with the vault key like its entries (`personal_search`), so the server never
  sees what someone looks for there. Picks are preferences, not audited.
- **Session tabs** (`web/src/lib/session/`): sessions open as tabs inside the page and keep running
  while the user moves between its pages; a reload ends them. `/connect/{id}` shows one session alone,
  for a window of its own.
- **Device journal** (`device_journal`, `api/journal.rs`): every opened connection with its purpose and
  end, and the notes people leave; only ever added to, readable by who may connect. Users and groups in
  `purpose_principals` (chosen by administrators, audited) state a purpose before every connection; the
  server refuses the connection without one, before anything reaches the device.
- Plaintext lives only in `secrecy`/`zeroize` types and never appears in logs, API responses (except the
  audited `reveal`), environment variables or command lines. Core dumps are disabled.

## 4a. Audit log

- Table `audit_log`, written in the same transaction as the action it records. A trigger numbers every
  entry without gaps (advisory lock) and stores the hash of its predecessor and its own SHA-256 over all
  fields; the chain starts from a hash of the installation id.
- Triggers refuse UPDATE, DELETE and TRUNCATE. A superuser can bypass them, but not unnoticed:
  `remotehub verify-audit` and `POST /api/audit/verify` recompute the chain and name the first broken entry.
- Actions have stable names (`session.sign_in` …), defined once in `crates/server/src/audit.rs`; the UI
  translates them (generated list, type-checked messages). Details never contain secrets.
- Administrators (members of `REMOTEHUB_ADMIN_GROUPS`, break-glass accounts) read the log in the UI.

## 5. Protocol engines

| Protocol | Engine | Browser | Recording (M5) |
|---|---|---|---|
| SSH | russh in `crates/gateway` | xterm.js | asciicast v2 on the server |
| RDP | guacd 1.6, FreeRDP 3 | Guacamole JS client (vendored from guacamole-client 1.6.0) | `.guac` on the server |
| VNC | guacd 1.6 | Guacamole JS client | `.guac` on the server |
| HTTPS | Chromium in the browser service, shown through guacd as VNC | Guacamole JS client | `.guac` on the server |

- SSH signs in with a stored password or key, asked credentials, the own account, or a certificate from
  remotehub's own CA (`crates/gateway/src/ssh_ca.rs`): a fresh Ed25519 key per connection, signed for the
  user's name as principal and valid for five minutes. Targets trust the CA's public key
  (`/api/ssh-ca.pub`); the CA key is a file like the master key.
- SSH and RDP can also sign in as the local administrator whose password LAPS keeps on the computer account
  (`crates/directory/src/laps.rs`): read with the service account at connection time, never stored.
- guacd runs in its own container without published ports, as non-root, read-only, with resource limits
  and a pinned version. It links GPL-licensed libvncclient and therefore stays a separate process.
- The guacd image (`deploy/guacd`) is built from the Apache source release (checksum pinned) against
  FreeRDP 3 from Debian, because the official image still links FreeRDP 2. It contains only the RDP and VNC
  plugins; FreeRDP gets a tmpfs home directory for its state.
- guacd 1.6 authenticates RDP with NTLM only; Kerberos arrives with guacd 1.7 (GUACAMOLE-2057). Members of
  *Protected Users* and domains without NTLM are not supported until then.
- RDP certificates and SSH host keys are pinned on first use; a change aborts the connection and shows both
  fingerprints. Someone with `edit` on the device can forget the pin (audited); changing its host, port or
  protocol forgets it too.
- **SSH WebSocket** (`/api/devices/{id}/terminal`): the handshake needs the session cookie, the own `Origin`
  (against cross-site WebSocket hijacking) and `connect`. The first frame gives the terminal size and, for
  devices that ask, the credentials for this connection only; then binary frames carry keystrokes and output.
  Every connection is audited as opened, closed (duration, bytes, exit code) or failed.
- **Display WebSocket** (`/api/devices/{id}/display`, RDP and VNC): the same checks. The first frame gives
  the display size and time zone (and asked credentials); the server opens the connection through guacd
  with the credentials in `connect` and answers `connected`. Then text frames carry whole Guacamole
  instructions; from the browser only input, display size, clipboard and stream acknowledgements reach
  guacd — `argv` (changing connection parameters), file transfer and pipes are dropped and counted in the
  audit entry. Own-account RDP signs in with the user principal name, which carries the domain.
- **HTTPS devices** (ADR 0007) use the display WebSocket too. remotehub pins the device's certificate like
  RDP's, then asks the browser service for a Chromium on the device: its own Xvnc display, a fresh profile,
  a proxy that reaches only the device's host and port, and the certificate's key as the only one accepted
  besides valid ones. The service types the credentials into the page's sign-in form through the DevTools
  Protocol; remotehub opens the display through guacd with the VNC password the service made up. One TCP
  connection to the service is the session: closing it ends Chromium.
- **Site connectors** (ADR 0008) reach devices in networks remotehub cannot reach. A connector
  (`remotehub connector`, `crates/server/src/connector_agent.rs`) keeps a control WebSocket open to
  `/api/connectors/control`; for each connection remotehub asks for, it connects to the device and opens a
  WebSocket of its own under `/api/connectors/streams/{id}`. The engines do not know: for a device with a
  connector, `connect::route` opens a forward (`crates/server/src/connectors.rs`) that only the engine in
  question may use, and hands its address to SSH, the certificate probes, guacd or the browser service.
- All engines sit behind the trait `ProtocolEngine`, so an own RDP engine (IronRDP) can replace guacd later
  without changing API or UI.

## 6. Internationalisation

English is the base locale, German the second; more can follow (ADR 0002).

- **UI:** paraglide-js with `web/messages/{en,de}.json`, typed message functions.
- **Server:** API errors are `ErrorCode`s with parameters, translated by the UI; server-rendered text
  (the admin CLI now, later exports and mails) uses Fluent catalogs via `Message`.
- **Guards:** tests fail on words in components, missing or unused keys, mismatched placeholders, German
  texts identical to English (unless allow-listed), error codes without messages, and Fluent catalogs whose
  message IDs or variables differ between locales.

## 7. Repository, development, operations

- `crates/`, `web/`, `migrations/`, `deploy/`, `docs/`, `scripts/ci/`.
- **Development** runs entirely in Docker (`deploy/compose.dev.yml`): PostgreSQL on `127.0.0.1:55440`, UI on
  `127.0.0.1:5180`, the server rebuilt by watchexec, plus a test lab (Samba AD DC, SSH target, a desktop
  target with RDP and VNC, a web target with a sign-in page), guacd and the browser service on the compose
  network.
- **CI** runs locally (`scripts/ci/lokal.sh`) and reports the commit status `lokal`; `main` requires it.
- **Operations:** three images on GHCR, `ghcr.io/hilman2/remotehub` (`deploy/Dockerfile`: the binary and the
  built UI on distroless, user 65532, read-only), `ghcr.io/hilman2/remotehub-guacd` (`deploy/guacd`) and
  `ghcr.io/hilman2/remotehub-browser` (`deploy/browser`: Chromium, Xvnc and the agent, user 10001,
  read-only). The ops package `deploy/ops` runs them with PostgreSQL: `compose.yml`, `init.sh` for `.env` and the secrets
  as files. PostgreSQL sits on an internal network that only remotehub reaches. Installing, backup and
  upgrades: [`docs/install.md`](install.md). The CI job `image` tries both images with the package
  (`scripts/ci/image-check.sh`); releases via `scripts/ci/release.sh`.

## 8. Milestones

| Milestone | Content |
|---|---|
| M0 Foundation | Workspace, local CI, development environment, SPA shell, i18n guards, ADRs |
| M1 First connection | LDAP sign-in, sessions, break-glass, folders/devices/grants, vault core, SSH in the browser, audit log |
| M2 RDP and VNC | guacd image, Guacamole tunnel, session tabs, clipboard, TOFU; first release 0.1.0 |
| M3 Vault | Reveal with audit, history, personal vault, generator, TOTP fields, attachments, KDBX import and export |
| M4 Identity | Entra ID via OIDC, second factor for directory sign-ins |
| M5 Accountability | Session recording and playback, audit export, clipboard and file policies |

Later ideas are GitHub issues with the label `backlog`.

## 9. Looking left and right

| Project | What we adopt | Why remotehub still exists |
|---|---|---|
| Apache Guacamole | guacd as RDP/VNC engine, its protocol and JS client | No vault; OIDC only via implicit flow; dated UI |
| Warpgate | Reference for server-side IronRDP rendering and russh | A bastion without a password vault, folder permissions by AD group or KDBX |
| Devolutions Server + Gateway | The feature set to aim for (credential injection, vault) | Proprietary |
| JumpServer | — | SSO and rotation only in the enterprise edition |
| Teleport | Smart-card emulation for passwordless RDP (later) | No vault; free edition restricted by license |
| KeePass / KeePassXC | KDBX as import and emergency format | Single key per file, no per-entry permissions |
