# Contributing

Thank you for your interest in remotehub. Contributions are licensed under the project's license
(AGPL-3.0, inbound = outbound).

## Workflow

1. **Issue first.** Every bug and every improvement starts as an issue, so the approach can be agreed on
   before code is written.
2. **One branch per issue, changes via pull request.** Reference the issue in the PR ("Closes #12").
   Commit titles are plain English sentences without a prefix.
3. **Checks run locally, not on GitHub Actions.** The maintainer runs `bash scripts/ci/lokal.sh --pr N`,
   which checks the PR's head commit in Docker and reports the commit status `lokal`. `main` only accepts
   pull requests with a green `lokal` status.

   The local CI mounts the Docker socket, so it effectively runs with root rights on the maintainer's
   machine. **PRs from forks are only checked after their diff has been read.**

## Rules that tests enforce

- **No hard-coded user-facing text.** The UI takes every string from `web/messages/{en,de}.json`,
  Rust from Fluent catalogs. Tests fail on text in components, missing or unused keys and mismatched
  placeholders. Add every new text in English and German.
- **Secrets never leave the vault unintentionally:** never in logs, API responses (except the audited
  `reveal`), environment variables or command lines.
- **Permissions only through `authorize()`.**

## Development

Everything runs in Docker; see `CLAUDE.md` for the commands (it doubles as the developer guide).
