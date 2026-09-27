# ADR 0017: A browser extension fills vault logins into web pages

- Status: accepted
- Date: 2026-09-27

## Context

Since ADR 0016 the vault holds the logins no device uses: web shops, licence portals, supplier accounts.
Signing in with one took three trips to the vault, for user name, password and one-time code, each through
the clipboard, and nothing checked that the page was the one the login belonged to. KeePass users have
KeePassXC-Browser for this (#201).

An extension that fills passwords into pages hands them to the page, and to any script on it. The questions
were when it may do so, into which pages, how it signs in to a remotehub with directory accounts, local
accounts, providers and second factors, and how it reaches browsers of a self-hosted installation.

## Decision

- **A Manifest V3 extension for Chromium browsers from version 127** (Edge, Chrome and others), built from
  `web/src/extension` with the web UI's messages, vault cryptography and error texts. Firefox and Safari
  are not covered.
- **It fills only when asked:** a click in its popup, its keyboard shortcut, or the context menu of a text
  field. Never when a page loads. A password leaves the server only when a user with `reveal` asks for it;
  filling on load would hand it to every matching page without anyone asking.
- **Only into pages the login belongs to** (`crates/server/src/site.rs`, `web/src/extension/site.ts`, one
  file of cases for both): same scheme and port, and over `https` the same registrable domain by the Public
  Suffix List; over `http`, and for addresses and hosts without a dot, the same host only. The top frame
  and frames of its origin are filled, frames of other origins never; only fields a person can see and
  type into. The server checks the page again before it hands out a password.
- **Nothing is drawn into pages.** A menu injected into a page can be covered and clicked by the page
  itself; choices happen in the popup, which is part of the browser.
- **Least privilege:** `activeTab` instead of access to every site, no `tabs` permission, host access only
  to remotehub's own address, asked for at setup. The extension matches pages against its list of logins
  itself, so remotehub does not learn which pages are open.
- **Connecting is OAuth 2.0's authorization code with PKCE.** `chrome.identity.launchWebAuthFlow` opens
  `/extension/connect`, where the user signs in as ever and allows the extension. The page sends a code,
  valid once and for a minute, to `https://<extension-id>.chromiumapp.org/`, which only that extension
  receives; remotehub hands codes only to the extension it serves and those in `REMOTEHUB_EXTENSION_IDS`.
- **The token is a session with the client `extension`** in `sessions`: idle time, maximum lifetime and
  directory changes (#108) end it like a browser's. It opens only the routes under `/api/extension/`, as a
  bearer token; a browser session opens none of them. *My account* lists and ends the extension's sessions.
  The extension keeps the token in `chrome.storage.session`, in memory only.
- **Filling and copying are reveals.** They take `reveal` and are audited as `credential.revealed` or
  `credential.code_shown`, with the purpose `fill` or `copy`, the page's origin and the client.
- **Personal logins stay end-to-end encrypted.** The extension unlocks them itself with the passphrase, the
  recovery key or a passkey; Chromium lets an extension use WebAuthn with the relying party of a host it
  may access, and the PRF output does not depend on the origin. The vault key lives in
  `chrome.storage.session` and is dropped after the session's idle time.
- **remotehub serves the extension itself,** under `/downloads/`, as the site connector (#188): a ZIP to load
  unpacked by hand, and a CRX signed with the maintainer's key with an update manifest, for browsers that
  install it by policy. The manifest carries the public key, so both have the same ID. There is no store.

## Consequences

- An extension loaded unpacked needs developer mode, does not update itself and is not signature-checked:
  whoever can write its folder changes what it does. Installing by policy avoids all three, but Chrome and
  Edge allow a CRX from outside their stores only on PCs in a domain or managed otherwise.
- The signing key decides the ID. Losing it means a new ID and a new policy on every PC that installs the
  extension that way. It lives outside the repository; `release.sh` refuses a release without it. The
  development key in `web/` is public, and only development and the CI allow its ID.
- Sign-in forms in frames of another origin, and password fields a page hides, are not filled. The user
  copies from the popup instead.
- Every fill of a shared login is a row in the audit log, like every reveal.
- Saving new logins from a page would need a script on every page; the extension does not.
