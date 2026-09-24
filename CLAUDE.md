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
4. **Delivery only as a release.** The version lives in the workspace `Cargo.toml` and is bumped in its own PR; then `bash scripts/ci/release.sh X.Y.Z` (arrives with M2).

In Git Bash, Claude sessions lack the Unix PATH: `gh` is at `/c/Program Files/GitHub CLI/gh.exe`, and `lokal.sh` needs it on the PATH: `export PATH="$PATH:/c/Program Files/GitHub CLI"`.

`scripts/ci/gemeinsam.sh`, the commit status `lokal` and the file name `lokal.sh` stay German on purpose: they are shared verbatim across all of the maintainer's repositories.

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
- `deploy/` holds the Dockerfiles, the development compose and (from M2) the ops package.
- `scripts/ci/` is the local CI: `lokal.sh` holds this repository's jobs, `gemeinsam.sh` is identical in all repositories and is not changed here.

## Commands

```bash
bash scripts/ci/lokal.sh               # local CI for HEAD, reports status "lokal"
bash scripts/ci/lokal.sh --pr 12       # check the head commit of a PR
bash scripts/ci/lokal.sh --help        # all options
```

The local CI needs Docker, `gh` and Git Bash. Only one run at a time per machine (lock under `~/.cache/ci-lokal`); logs are under `.git/ci-lokal/protokolle/`. Start longer runs in the background in Claude sessions.

**Ports on the development machine:** other projects already use 5173, 8025, 55432 and 55433. remotehub publishes only `127.0.0.1:5180` (UI) and `127.0.0.1:55440` (database); everything else stays on the compose network.

In Git Bash, set `export MSYS_NO_PATHCONV=1` before `docker compose` if needed, otherwise Git Bash rewrites paths.
