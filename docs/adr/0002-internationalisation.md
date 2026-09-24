# ADR 0002: Multilingual from the start

- Status: accepted
- Date: 2026-09-24

## Context

English is the project language and the UI's default; German is the second language, and more may follow.
Retrofitting translations later means touching every string twice, so remotehub is multilingual from day
one — and tests must fail when untranslated text slips in.

User-facing text comes from the UI itself, from API errors, and from text the server renders (exports,
notifications, audit descriptions).

## Decision

### UI: paraglide-js

- Messages live in `web/messages/{locale}.json` (inlang message format) and compile to typed functions
  (`m.nav_devices()`); a missing key is a type error in `svelte-check`.
- The locale comes from the user's choice (stored), then the browser, then English. It is never part of
  the URL. Switching reloads the page.
- Numbers and dates are formatted with `Intl` in the active locale (`formatLocale()`).

### API errors: codes, not sentences

- Errors are RFC 9457 problem responses with a stable `ErrorCode` and typed parameters. The UI turns them
  into text with the message `error_<code>`.
- `ErrorCode` is defined once in Rust (`crates/server/src/api/problem.rs`). The list of codes and the
  `Problem` type are generated into `web/src/lib/api/generated/problem.ts`; a Rust test fails when the
  checked-in file is stale. The UI indexes its messages with every code, so a missing message is a type
  error in `svelte-check`.

### Server-rendered text: Project Fluent

- Every user-facing string in Rust is a `Message` — key plus typed arguments — never a `String`.
- Catalogs live in `crates/i18n/locales/{locale}/*.ftl` and are embedded in the binary (arrives with the
  first server-rendered text, #27). Fallback chain:
  requested locale → English.

### Guards (tests fail on untranslated text)

- **UI** (`web/src/lib/i18n-guard.test.ts`, `web/src/lib/i18n.test.ts`):
  - Every `.svelte` file is parsed with the Svelte compiler; visible text and user-facing attributes
    (`title`, `alt`, `aria-label`, `placeholder` …) must not contain words. Only the product name is exempt.
  - All locales have the same keys, no message is empty, every key is used.
  - Every key has the same placeholders in every locale.
  - A German message identical to the English one fails unless allow-listed (e.g. "SSH", "Server").
  - Every `ErrorCode` has a message in every locale.
- **Rust** (`crates/i18n`): all catalogs have the same message IDs and the same variables per message
  (parsed with `fluent-syntax`), and every message renders in every locale without errors.

## Consequences

- Every PR with new user-facing text adds it to every locale (checklist in the PR template); CI fails
  otherwise.
- Two message formats to know (inlang JSON for the UI, Fluent for Rust); both are small.
- A pseudo-locale run with Playwright can later catch text that no static guard sees.
