# ADR 0012: The customer opens access per device and per group

- Status: accepted
- Date: 2026-09-26

## Context

Under ADR 0011, access at the connector is all or nothing. Open means remotehub reaches every port of every
address in `REMOTEHUB_CONNECTOR_ALLOW`. For a maintenance window a customer wants to open the three servers
of one application for two hours, and nothing else (#180).

remotehub could send its device list to the connector, and the customer would pick from it. Then remotehub
names what the customer opens, and a compromised remotehub could rename a device to widen what is open.

## Decision

- **The customer keeps their own list** in the connector (`inventory.json`): devices with a name, an
  address (IP, range or host name) and ports, and groups of devices. A device can be in several groups. The
  connector takes nothing of it from remotehub.
- **Each device and each group has its own access**, closed, open until a point in time, or open without
  end, stored next to the whole network's in `access.json`. The switch of ADR 0011 stays as "whole network"
  with its meaning.
- **The connector decides for every stream.** It resolves the target itself, host names included, and lets
  it through to a port of an open device, or anywhere while the whole network is open.
  `REMOTEHUB_CONNECTOR_ALLOW` stays the outer limit. When a device closes or leaves the list, its running
  connections end.
- **The control socket is up while anything is open.** The state report says `partly` when only devices or
  groups are open, and lists the open groups and devices with their address, ports and end. remotehub
  refuses a report whose lists are longer than 1000 entries, hold anything but short plain text, or name an
  end that is no point in time. Only administrators see the lists.
- **remotehub asks before it connects.** For a connector that is open in part, `connect::route` sends a
  `Check` with the target on the control socket and refuses a closed one with `connector_target_closed`,
  before engines, credentials or pins are involved. A stream the connector still refuses is reported as not
  open.
- **A device's own state comes from the connector, too.** The report's list names the customer's devices,
  not remotehub's, and a host name there may resolve only in the customer's network. So the device's page
  and a session ask with `Check`, whose answer carries the latest end of what opens the target.
- **The journal names the device or group** in each opening, closing and expiry, and the list's changes.

## Consequences

- A compromised remotehub reaches, while access is open in part, only the ports of the open devices.
- The service provider sees the customer's list of open devices. What is closed stays with the customer.
- A connection in part costs one more round trip to the connector, and each device's page and session one
  a minute.
- Lines of the journal written before this change mean the whole network.
