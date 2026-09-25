# CLAUDE.md

Guidance for Claude Code in this repository.

**Language:** English for everything in the repository and on GitHub — code, identifiers, comments, docs, issues, pull requests and commit messages. All user-facing text is multilingual (English default, German). Conversation with the product owner may be in German.

## What remotehub is

remotehub is a self-hosted, browser-based remote access manager and credential vault for administrators — an open-source take on RDP managers plus KeePass.

- **Server** (Rust, one binary): API, sign-in, permissions, vault, audit, SSH engine and the Guacamole tunnel; serves the UI.
- **Protocols run on the server.** SSH via russh; RDP and VNC via guacd (sidecar container on the internal network). The browser only displays and sends input; it never needs a route to the targets and never sees stored credentials.
- **Sign-in:** Active Directory via LDAP first, Entra ID (OIDC) later, behind one `IdentityProvider` trait.
- **Database:** PostgreSQL (sqlx, forward-only migrations at startup).
- **Frontend:** SvelteKit with Svelte 5 as a static SPA.
- **Everything runs in Docker.** One organisation per instance.

The architecture is in `docs/architecture.md`, the reasoning in `docs/adr/`.

## Working agreement (mandatory)

Everything goes through the GitHub workflow of the public repository `hilman2/remotehub`, for every bug and every improvement, however small:

1. **Issue first.** Every bug and every improvement gets a GitHub issue before code is touched.
   - Templates "Bug report" and "Feature request", labels `bug`, `enhancement`, `documentation`, `security`.
   - Every issue belongs to the milestone of its stage (M0–M5); later ideas carry no milestone and the label `backlog`.
   - Decisions are recorded in the issue.
2. **One branch per issue, changes via pull request.**
   - Branches are named `<nr>-<short-name>`. No commits directly on `main` (ruleset). The PR references the issue ("Closes #12").
   - Commit titles are plain English sentences without a prefix.
   - Checks run **locally, not on GitHub Actions**: commit, push, then `bash scripts/ci/lokal.sh --pr N` in Git Bash. The script checks the commit in Docker and reports the status `lokal`, which the ruleset requires to be green.
   - There are no Actions workflows; self-hosted runners were rejected (tenderhub #106).
   - **PRs from forks:** read the diff before running `lokal.sh --pr N` — the CI mounts the Docker socket.
3. **Close the issue after merging** ("Closes #N" does it; otherwise `gh issue close N --comment "Done in #PR."`).
4. **Delivery only as a release.** The version lives in the workspace `Cargo.toml` and in `deploy/ops/.env.example` (`REMOTEHUB_VERSION`); both are bumped in their own PR. Then, on the merged `main` with a green `lokal`: `bash scripts/ci/release.sh X.Y.Z` (`--dry-run` builds and tries without publishing). It pushes both images to GHCR and creates the GitHub release with the ops package; it needs `docker login ghcr.io` with a token that may write packages.

In Git Bash, Claude sessions lack the Unix PATH: `gh` is at `/c/Program Files/GitHub CLI/gh.exe`, and `lokal.sh` needs it on the PATH: `export PATH="$PATH:/c/Program Files/GitHub CLI"`.

This repository shares nothing with other projects. `scripts/ci/gemeinsam.sh` started as a copy from another repository and belongs to remotehub alone; change it freely here (its German comments are a leftover). The CI lock is per repository (`.git/ci-lokal/sperre`).

## Principles

- **Credentials never reach the browser** unless a user with `reveal` asks to see them (audited). Connections get them injected on the server.
- **`authorize()` in `crates/model` is the only place** that decides permissions. Roles `list < connect < reveal < edit < manage`, granted to AD groups or users on folders or entries, inherited downwards, allow-only.
- **Identities are immutable IDs** (`objectSid`/`objectGUID`, later Entra object IDs), never names.
- **Secrets:**
  - Encrypted per entry version (XChaCha20-Poly1305) with a data key wrapped by a master key from a file (Docker secret), never from `.env`.
  - Never in logs, API responses (except `reveal`), environment variables or command lines.
  - Use `secrecy`/`zeroize` types for plaintext in memory.
- **Every reveal, connect and export is audited** in an append-only, hash-chained log.
- **No hard-coded user-facing text** (ADR 0002), and tests fail if any slips in.
  - **UI:** add every string to `web/messages/en.json` and `de.json` and use `m.<key>()`. The guard test parses all `.svelte` files and fails on words in markup or in `title`, `alt`, `aria-label`, `placeholder` …; it also fails on key mismatches between locales, empty messages, unused keys and mismatched placeholders.
  - **API errors** are RFC 9457 problems carrying an `ErrorCode` plus parameters; the UI translates them, and a test fails if a code lacks a message in any locale.
  - **Rust:** user-facing strings are `Message`s (key + typed arguments) with Fluent catalogs per locale, never `String`s.
  - **Formatting:** numbers and dates go through `Intl` with `formatLocale()`, never with a fixed locale.
- **A state is never colour alone** — always icon and text.

## Layout

Built up with the issues of milestone M0:

- `crates/` holds `server`, `model`, `vault`, `directory`, `gateway` and `i18n`.
- `web/` is the SvelteKit frontend.
- `migrations/` holds the sqlx migrations.
- `deploy/` holds the Dockerfiles (`Dockerfile` is the production image), the development compose and the ops package `deploy/ops` (see `docs/install.md`).
- `scripts/ci/` is the local CI: `lokal.sh` holds this repository's jobs, `gemeinsam.sh` is identical in all repositories and is not changed here.

## Commands

Development happens entirely in Docker (`deploy/compose.dev.yml`):

```bash
docker compose -f deploy/compose.dev.yml up -d --build     # db, server, web
docker compose -f deploy/compose.dev.yml logs -f server
docker compose -f deploy/compose.dev.yml run --rm workbench cargo nextest run
docker compose -f deploy/compose.dev.yml run --rm workbench cargo clippy --workspace --all-targets
docker compose -f deploy/compose.dev.yml run --rm --no-deps web pnpm check   # also: lint, test, build, format
docker compose -f deploy/compose.dev.yml down               # volumes are kept
```

- **Server** is rebuilt and restarted on every change under `crates/` or `migrations/`. On Windows, file events from bind mounts do not reach containers, so `watchexec` polls (`--poll`).
- **Build output:** `target/` lives in the volume `target` (path `/target`), not in the working copy. The CI and development images link with mold (`CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS`).
- **Database:** `127.0.0.1:55440` (user, password and database `remotehub`). Tests with `#[sqlx::test]` create a fresh database per test on the same server.
- **Configuration:** `REMOTEHUB_*` environment variables, each also as `REMOTEHUB_*_FILE` (Docker secrets); see `crates/server/src/config.rs`. Required: `REMOTEHUB_DATABASE_URL`, `REMOTEHUB_PUBLIC_URL` (the origin people open; state-changing requests from other origins are refused), `REMOTEHUB_MASTER_KEY_FILE` (only as a file; create a line with `remotehub generate-key`, the development key is `deploy/dev/master.key`). Directory: `REMOTEHUB_LDAP_URL`, `_BIND_DN`, `_BIND_PASSWORD`, `_BASE_DN`, optional `_CA_FILE`, `_STARTTLS`, `_USER_FILTER`. Connecting with the own account: `REMOTEHUB_OWN_ACCOUNT_CONNECTIONS` (default `true`). Administrators: `REMOTEHUB_ADMIN_GROUPS` (SIDs or group names, looked up at startup; dev: `RH Admins`, i.e. alice). RDP and VNC: `REMOTEHUB_GUACD` (`host:port`, default `guacd:4822`). SSH certificates: `REMOTEHUB_SSH_CA_KEY_FILE` (only as a file; `remotehub generate-ssh-ca`; dev and CI use the lab's `deploy/testlab/ssh/remotehub_ca`). All settings: `docs/configuration.md`.
- **CLI:** `remotehub generate-key`, `remotehub verify-audit` (exits 1 if the audit chain is broken), `remotehub break-glass create|reset|delete|list` (emergency accounts; password and TOTP secret are printed once); `remotehub` alone serves. In development: `docker compose -f deploy/compose.dev.yml exec server cargo run -q -p remotehub-server -- break-glass create emergency`.
- **Signing in during development:** http://localhost:5180 with a test lab user, e.g. `alice` / `Alice-Passw0rd!`. The session cookie is `Secure`; browsers accept it on `localhost`, curl only on `localhost` (not `127.0.0.1`).
- **UI:** http://localhost:5180, Vite with HMR via polling; `/api` is proxied to the server. `node_modules` lives in the volume `web-node-modules`, not in the working copy.
  - pnpm only takes versions older than seven days (`web/pnpm-workspace.yaml`), so don't pin a range to a brand-new release.
  - Messages are in `web/messages/{en,de}.json`; `pnpm i18n` compiles them (also part of `pnpm check` and `pnpm test`). Language and theme switch sit in the header.
- **Test lab** (`deploy/testlab/`): a Samba AD domain controller `dc` (domain `REMOTEHUB.TEST`, LDAPS with the test CA in `deploy/testlab/dc/tls/ca.crt`, users and nested groups in `users.sh`), an SSH target `ssh-target` and a desktop target `desktop-target` with RDP and VNC (see their READMEs), plus `guacd` from `deploy/guacd`. All run on the compose network only; guacd is hardened as in production.
  - The server's integration tests are **one** test binary, `crates/server/tests/api/` (add new files as modules in `main.rs`): every file directly in `tests/` would be linked into its own binary with the whole server in it. Helpers are in `tests/api/common.rs`.
  - Tests that need the lab are marked `#[ignore = "needs the test lab"]` and read `REMOTEHUB_TEST_LDAP_URL` and `REMOTEHUB_TEST_SSH_HOST`. Run them with `docker compose -f deploy/compose.dev.yml run --rm workbench cargo nextest run --run-ignored only`. The CI runs them after the normal tests, with the lab started while Rust compiles. Use `#[ignore]` for nothing else.
  - The test lab's passwords, keys and certificates are public on purpose and protect nothing else.
- **End-to-end tests** (Playwright, `web/tests/e2e/`) run in a real browser against the running development stack: `docker compose -f deploy/compose.dev.yml --profile e2e run --rm e2e`. The CI runs them in full runs, for changes under `web/src/` or to the tests, and with `CI_E2E=1`. Keep `@playwright/test` equal to the Playwright image in `compose.dev.yml` and `lokal.sh`.
- **rust-analyzer** runs via the dev container (`.devcontainer/`, service `workbench`).
- **Rust version:** it appears in `rust-toolchain.toml`, `scripts/ci/tools.Dockerfile` and `deploy/dev/rust.Dockerfile`; the CI job `base` checks that all three match.

```bash
bash scripts/ci/lokal.sh               # local CI for HEAD, reports status "lokal"
bash scripts/ci/lokal.sh --pr 12       # check the head commit of a PR
bash scripts/ci/lokal.sh --help        # all options
```

The local CI needs Docker, `gh` and Git Bash. Only one run of this repository at a time (lock `.git/ci-lokal/sperre`); other repositories never wait for it. Logs are under `.git/ci-lokal/protokolle/`.

- **Jobs:** `base` (always, seconds), `code`, which checks Rust (fmt, clippy, tests, then the lab tests) and the web UI (svelte-check, lint, vitest, build) in parallel, and `image`, which builds the production images and tries them with the ops package (`scripts/ci/image-check.sh`) when `IMAGE_INPUTS` change and in full runs.
- **Nothing is checked twice:** a commit with the same file state (Git tree) as one already checked green — e.g. a PR's merge commit — takes over that result without running.
- **Only what changed is compiled:** Rust builds from the fixed path `/src` (volume `remotehub-ci-src`), synced by content, so cargo rebuilds only the crates whose files changed.
- **Only what a change can affect runs:** the files changed since the merge base with `main` decide (`RUST_INPUTS`, `WEB_INPUTS`, `LAB_INPUTS` in `lokal.sh`). A commit on `main` itself, a change under `scripts/ci/` or `CI_FULL=1 bash scripts/ci/lokal.sh` runs everything.
- **Working rhythm (Claude sessions):** while developing, run only the checks for what you are changing (one crate's tests, one vitest file, `pnpm check`). Run `lokal.sh` exactly once per PR, in the background, and continue with the next issue meanwhile — never the full suite by hand and then again in the CI.
  - While a `lokal.sh` run is in progress, do not switch branches or edit `scripts/ci/` in this working copy: bash reads the running script from disk, so a changed `lokal.sh` changes the run. Continue on a branch whose `scripts/ci/` is identical, or in a `git worktree`. The commit under test itself is safe; it lives in the run's own volume.

**Ports on the development machine:** other projects already use 5173, 8025, 55432 and 55433. remotehub publishes only `127.0.0.1:5180` (UI) and `127.0.0.1:55440` (database); everything else stays on the compose network.

**Docker and the office network:** Docker's default bridge uses `172.17.0.0/16`, and so does the office LAN (e.g. `172.17.0.90`). With the default, containers — guacd above all — cannot reach LAN hosts in that range: the traffic stays on Docker's own bridge. On such a machine, move Docker's ranges in Docker Desktop → Settings → Docker Engine (`%USERPROFILE%.dockerdaemon.json`):

```json
"bip": "10.211.0.1/24",
"default-address-pools": [{ "base": "10.212.0.0/16", "size": 24 }]
```

Then restart Docker Desktop (this stops every container, other projects' included) and recreate the development network with `docker compose -f deploy/compose.dev.yml down` and `up -d`; existing networks keep their old range until they are recreated. Check with `docker network inspect bridge` and a connection test from the guacd container.

In Git Bash, set `export MSYS_NO_PATHCONV=1` before `docker compose` if needed, otherwise Git Bash rewrites paths.
