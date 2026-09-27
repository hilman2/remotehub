# ADR 0015: Devices sign in with login profiles, not with vault entries

- Status: accepted
- Date: 2026-09-27

## Context

After ADR 0014, shared credentials lived in collections, and a device signed in with one of them. The
vault still listed device logins next to web shop accounts, and a device pointed into the vault. The
product owner's model is two tools side by side: the devices area works like mRemoteNG, the vault like
KeePass (#192).

## Decision

- **A device signs in with what the devices area holds:** its own credentials, a login profile, or the
  modes without a stored secret (ask, own account, certificate, LAPS). No device uses a vault entry, and
  the vault knows no device.
- **A login profile is a named login many devices share**, a password or an SSH key. Changing it changes
  it for every device that uses it.
- **A profile lies in a device folder and has that folder's grants.** It takes no grants of its own
  (`ObjectId::Profile`, its container is the folder). A profile at the top level is for administrators
  only.
- **A device uses only a profile in its folder or one above.** A customer's profile stays with that
  customer's devices. Moving a folder or a profile so that a device would lose its profile is refused, and
  so is deleting a profile a device still uses.
- **The rules that kept vault entries from leaking hold for profiles.** Linking a profile, or giving a
  device that uses one another target, needs `connect` on the profile. Showing its secret needs `reveal`
  and is audited. A key profile is for SSH devices only.
- **Vault entries are passwords.** The SSH key kind leaves the vault; keys for devices live in profiles.
- **No migration of data:** nothing is live yet. Devices that used a vault entry ask instead.

## Consequences

- Access to a server and to its login are granted together again, by the device folder. The vault's
  collections are only for what no device uses.
- A login shared across customers' folders is either a top-level profile, for administrators, or one
  profile per customer.
- ADR 0014 keeps its collections; its link from devices into the vault is replaced by this ADR.
