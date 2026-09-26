# ADR 0011: The customer opens and closes access at the connector

- Status: accepted, extended by [ADR 0012](0012-access-per-device.md) (access per device)
- Date: 2026-09-26

## Context

A site connector (ADR 0008) lets remotehub reach a customer's network whenever remotehub wants. NIS2 and
cyber insurers ask that remote access by a service provider be granted by the customer, limited in time and
logged on the customer's side (#165).

The switch could live in remotehub: a customer account there that opens and closes access. Then the service
provider's system decides whether it may enter the customer's network. A break-in at the provider reaches
every customer whose access is open, and the customer cannot tell from remotehub's own audit.

## Decision

- **The switch is in the connector.** Access is closed, open until a point in time, or open without end.
  It starts closed. The state lives in the connector's data directory (`access.json`).
- **Closed means no connection to remotehub.** A closed connector opens no control socket, so no request of
  remotehub reaches code that could act on it. Closing, and the time running out, end running connections
  at once.
- **The connector still reports.** Every minute and on each change it posts its state to
  `/api/connectors/state` and reads nothing from the answer but the status. remotehub shows "closed by the
  customer" instead of "not connected", refuses connections with `connector_closed`, warns in a session ten
  minutes before the end, and records each change in its audit log.
- **The customer switches in the connector's own web interface**, over HTTPS, with users that exist only on
  the connector (password, optionally TOTP; five failed attempts lock a name for five minutes), or on the
  connector's command line. Both write through the same function; the running connector reads the file
  every second, so a change on the command line reaches it too.
- **The customer keeps a journal** (`access.log`, JSON lines, and the log output): who opened and closed
  access, and every connection with target, remotehub user, duration and bytes. remotehub names the user
  in each `open`; the journal marks it as reported by remotehub, since the connector cannot check it.
- **The connector depends on no other remotehub crate but `i18n` and `totp`**, which have no native code,
  so it still builds for Windows (#166).

## Consequences

- A compromised remotehub reaches no customer whose access is closed. While access is open, the allowed
  address ranges (`REMOTEHUB_CONNECTOR_ALLOW`) and the pins of ADR 0008 still apply.
- The connector needs a writable data directory, a volume in Docker, and publishes a second port for its
  web interface. Who reaches that port is the customer's choice.
- remotehub knows the state only as reported. A connector that stops reporting shows as "not reported"
  after three minutes.
- Maintenance windows and a request from remotehub to the customer are left for later.
