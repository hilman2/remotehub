# ADR 0014: Shared credentials live in collections

- Status: accepted
- Date: 2026-09-27

## Context

Under ADR 0004, shared credentials lived in the folder tree beside the devices. A folder of servers held
their passwords, and a grant on the folder reached both. The vault showed shared credentials as a second
list below the personal one, sorted by folder path.

That mixes two things. A credential for a web shop, a hosting account or a supplier portal belongs to no
device, yet it needed a device folder to live in. The devices page listed credentials between devices, and
the vault had no order of its own for them (#190).

## Decision

- **Collections are a tree of their own**, apart from the device folders: `collections` with a parent and a
  name. Every credential lies in exactly one collection (`credentials.collection_id`); folders hold only
  devices.
- **Grants work on collections as on folders.** A grant names a folder, a device, a credential or a
  collection. It is inherited downwards within its own tree, and only there: a grant on a folder reaches no
  credential. `authorize()` stays the single decision point; `Catalog::container()` names the parent of
  each kind.
- **A device only refers to its credential.** Whoever may connect to the device connects with it, as
  before; the password still never reaches the browser.
- **Managing collections takes `manage`** on the collection above; the top level is for administrators. A
  collection is deleted only when empty. Every change is audited.
- **The vault is the one place for credentials.** It lists personal entries and shared credentials
  together, narrowed by personal folder, collection or kind, searched across both, sorted by name, use or
  place. The devices page shows the credential a device signs in with and links to it in the vault.
- **Migration:** every folder that held credentials, and the folders above it, becomes a collection with
  the same name, place and ID, and keeps its grants. Nobody loses access, and nobody gains any.
- **Access requests stay per credential.** A collection cannot be requested.

## Consequences

- A migrated collection has the same ID as the folder it came from. Code that looks up grants or names must
  tell them apart by kind, never by the bare ID.
- Access to a server and to its password are now granted separately. After the migration they match; later
  grants on a folder no longer reach new credentials, which is the point.
- The personal vault stays in the browser (ADR 0004); the shared collections work without it being
  unlocked.
