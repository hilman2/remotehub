# ADR 0016: The vault works like KeePass

- Status: accepted
- Date: 2026-09-27

## Context

After ADR 0015, the vault holds only what no device signs in with: web shops, licence portals, supplier
accounts. Its page was still a list with a detail panel, built for picking a device's credential. People
who keep such logins know KeePass, and compared with it the page lacked most of what they use every day:
columns to sort by, a recycle bin, expiry dates, tags, one-time codes next to the password, keyboard
shortcuts, a context menu (#193).

## Decision

- **One window as in KeePass.** A tree on the left (views *All*, *Recent*, *Expiring*; the personal
  folders and their recycle bin; the shared folders and theirs), a table with sortable columns in the
  middle, the chosen entry below it. Copying, opening, editing, moving and deleting work from the
  toolbar, the context menu, the keyboard and by dragging. The UI calls collections *shared folders*.
- **Credentials carry what KeePass entries carry.** Tags, a day the password runs out, and a TOTP secret
  join URL, notes, icon, fields and files. The separate domain goes: a vault login keeps its domain in
  the user name, and devices sign in with login profiles.
- **Shared one-time codes are made on the server.** The TOTP secret is sealed as the field `totp` with the
  credential's version, like any protected field. `POST /api/credentials/{id}/code` returns the current
  code; it takes `reveal` and is audited (`credential.code_shown`) like a reveal. The secret itself leaves
  the server only in an export. A personal entry keeps its secret in its sealed content, and the browser
  makes the code.
- **Deleting is two steps.** A delete moves a credential into its side's recycle bin (`deleted_at`); it
  keeps its grants and can be restored. Only a purge (`?purge=true`), asked for in the bin, removes it
  for good, with its sealed versions and files, so a repeated request cannot. Both take `edit`. Deleting
  a collection purges its bin first. A personal entry is marked inside its sealed content, so the server
  does not learn what its owner binned.
- **Moving between personal and shared goes through KDBX in the browser**, as import and export do: the
  entry is revealed, written as a KeePass entry, and created on the other side. The original is removed
  only once the copy exists.
- **A vault entry is asked for with `reveal` only.** `connect` means nothing for it.

## Consequences

- Nobody can read a shared TOTP secret without an export, which needs `reveal` on every entry and is
  audited. A code shown on screen is valid for a period at most.
- A binned credential is still in the database and in backups until it is deleted a second time.
- An entry moved from the shared side to the personal one leaves the shared audit trail: from then on
  the server sees only ciphertext.
