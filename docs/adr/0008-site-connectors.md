# ADR 0008: Site connectors carry TCP; the engines stay with remotehub

- Status: accepted
- Date: 2026-09-25

## Context

Devices in other sites or network segments are often out of remotehub's reach, and opening inbound firewall
rules for each is what administrators want to avoid. #19 asked for a small connector per network that opens an
outbound tunnel to remotehub "and runs the protocol engines there".

Engines at the site would mean guacd and the browser service (ADR 0007) in every site: C code and Chromium to
harden and update in places that are harder to reach, and a second protocol between remotehub and each site's
engines.

## Decision

- **The connector only carries TCP.** It is a binary of its own, `remotehub-connector` (crate
  `crates/connector`, image `ghcr.io/hilman2/remotehub-connector`), that depends on no other remotehub crate
  but `i18n` and `totp`, so it builds without the server and for Windows too (#164). It connects only while
  the customer keeps access open (ADR 0011). It started as `remotehub connector` in the
  server's image. It keeps a control WebSocket open to remotehub's public address, signed in with a token and
  its protocol version; remotehub refuses any other version. When an
  engine needs a device behind it, remotehub sends `open` with the device's host and port; the connector
  connects there and opens a second WebSocket to remotehub for this connection's bytes.
- **One WebSocket per TCP connection**, not a multiplexer: every connection keeps TCP's own flow control, and a
  slow RDP session cannot hold up an SSH session of the same site. The price is a WebSocket handshake per
  connection.
- **The engines stay with remotehub.** For a device behind a connector, remotehub opens a forward: a listener
  that carries every connection it accepts through a new stream. It accepts only the engine that needs it:
  loopback for SSH and the certificate probes, guacd's or the browser service's address for those. The engines
  get the forward's address as they get a device's; the browser service keeps the device's name for Chromium
  and only connects elsewhere.
- **Devices name their connector** (`devices.connector_id`). Changing it counts as changing the target: the pins
  are forgotten, and a linked credential needs `connect`, because the same address behind another connector
  may be another machine. Since #176 a folder names one for the devices below it; a device takes the nearest
  one above it unless it names its own or says `direct`. Changing a folder's connector, or moving a folder or
  device under another one, changes the target of every device affected, with the same consequences.
- **Tokens** are 32 random bytes, shown once, stored as SHA-256. Administrators create and delete connectors;
  a connector with devices cannot be deleted. The connector may be limited to address ranges
  (`REMOTEHUB_CONNECTOR_ALLOW`).

## Consequences

- A site needs one container and outbound HTTPS to remotehub, nothing inbound. The reverse proxy must pass
  WebSockets under `/api/connectors/`, as it does for sessions.
- RDP, VNC and the web interfaces' HTTPS cross the tunnel as their own protocols, not as Guacamole
  instructions. RDP is built for wide-area links.
- Whoever holds a token can pose as that connector and receive the connections remotehub opens to its site.
  The pinned host keys and certificates of SSH, RDP and HTTPS devices stop such a connection; VNC has no pin.
  A stolen token is revoked by deleting and recreating the connector.
- remotehub sees which connections a connector carries, now and since it started; the connectors page shows
  both.
