# ADR 0004: Vault with own data model and envelope encryption

- Status: accepted
- Date: 2026-09-24

## Context

remotehub replaces KeePass for teams of administrators. A KeePass database (KDBX) would be convenient as
storage, but it is one file encrypted with one key: whoever may open it sees everything, there are no
permissions per folder or entry, and concurrent changes can only be merged file by file.

Stored credentials must also be injected into sessions without reaching the browser (ADR 0003), so the
server has to be able to decrypt them.

## Decision

- **Own data model in PostgreSQL.** Entries (credentials, and devices referencing them) live in a folder
  tree with grants per folder or entry (ADR 0005 for identities).
- **KDBX only at the edges:** import of existing databases (M3), and an encrypted KDBX 4 emergency export
  for when remotehub itself is unavailable. Exports always go into a fresh file.
- **`connect` and `reveal` are separate permissions.** Using a credential for a session does not show it;
  showing it requires `reveal` and is audited.
- **Envelope encryption:**
  - Each version of an entry gets its own random data key; fields are encrypted with XChaCha20-Poly1305.
  - Associated data binds scheme, entry ID, field ID and key version, so ciphertexts cannot be moved
    between rows or fields.
  - Data keys are wrapped by a versioned master key from a `KeyProvider`. First provider: a key file
    mounted as a Docker secret (never an environment variable). Later: Vault/OpenBao Transit, Azure Key
    Vault, PKCS#11.
  - Every row stores `scheme`, `kek_id` and `kek_version`, so the master key can be rotated lazily.
- **Plaintext hygiene:** only in `secrecy`/`zeroize` types; never in logs, API responses (except `reveal`),
  environment variables or command lines; core dumps disabled.
- **Audit:** every reveal, connect and export is written to an append-only, hash-chained log.

## Consequences

- No zero knowledge for shared entries: whoever controls the server and its key file can decrypt them.
  This is the same trade-off every credential-injecting PAM product makes; it is stated in SECURITY.md.
- Personal entries can later get an end-to-end scheme (`e2e_user_v1`, key from WebAuthn PRF); the schema
  reserves room for it.
- The KDBX crate (`keepass`) is young and changes fast; it is isolated in `crates/kdbx` and pinned exactly.
