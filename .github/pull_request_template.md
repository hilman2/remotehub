## What changes?

<!-- One to three sentences. Reference the issue: "Closes #12". -->

## Why?

## Checked

- [ ] `bash scripts/ci/lokal.sh --pr N` is green and reported the status `lokal`
- [ ] Schema change: migration read, forward-only
- [ ] Secrets touched: never logged, never returned by the API (except `reveal`), never in env or command lines
- [ ] Permissions touched: only through `authorize()`, with table-driven tests
- [ ] New user-facing text: added to every locale (UI messages, Fluent catalogs)
- [ ] Architecture touched: `docs/architecture.md` or an ADR updated in the same PR
