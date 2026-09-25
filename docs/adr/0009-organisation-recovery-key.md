# ADR 0009: An organisation recovery key for personal vaults

- Status: accepted
- Date: 2026-09-25

## Context

The personal vault is encrypted in the owner's browser (ADR 0004). Nobody else can open it, and that is the
point. It is also a risk for a company: a vault is lost for good when its owner forgets passphrase and
recovery key, or leaves. Company passwords kept there go with them.

The server must not be able to open a personal vault on its own. Otherwise a stolen database with the
master key would be enough, and the end-to-end encryption would be a label.

## Decision

- **One key pair for the organisation.** An administrator creates it in the browser: ECDH on P-256. The
  server keeps only the public key. The private key leaves the browser twice: as a file sealed with a
  passphrase of its own (PBKDF2, AES-GCM), and as printable text for a safe (the 32-byte scalar in base32).
- **Every vault is wrapped for it as well.** The owner's browser wraps the vault key for the newest public
  key: an ephemeral ECDH key, HKDF-SHA-256, AES-KW. It stores the result as one more way to unlock, of kind
  `organisation`, at setup and at the next unlock of an older vault. The owner cannot remove it. The
  administrators see which vaults are covered.
- **A recovery needs two people.** An administrator asks for it, with a reason. A security officer
  (role of #106), who is not the same person, approves it. For a day after that, the administrator who
  asked may read the wrapped vault key and the sealed entries. Every step is audited.
- **The private key never reaches the server.** The recovery happens in the administrator's browser:
  - A forgotten passphrase: the browser writes a new recovery key for the vault, marked as one-time, and
    shows it once. The owner unlocks with it and has to choose a new passphrase and recovery key.
  - Leaving the company: the browser opens the entries and creates them as credentials in a shared folder,
    through the normal API.
- **Rotation:** a new key pair; vaults are wrapped for it at their next unlock. An old key can be deleted
  once no vault depends on it.

## Consequences

- Without the private key, a lost vault stays lost. Neither the server nor its operators can open one.
- Whoever holds the private key and an approved recovery can read that one vault. The approval and the
  audit log are the control; the key belongs in a safe, not on a desk.
- The wrap for the organisation is made by the owner's browser. The server cannot check that it opens the
  vault; a manipulated browser could store a useless one. The administrators see a vault as covered as soon
  as it holds a wrap for the newest key.
- A vault that was never unlocked since the key was created is not covered.
