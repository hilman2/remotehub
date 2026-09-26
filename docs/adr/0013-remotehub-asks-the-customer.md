# ADR 0013: remotehub asks the customer for access

- Status: accepted
- Date: 2026-09-26

## Context

Under ADR 0011 a closed connector reads nothing from remotehub but the status of its report. A technician
who needs a customer's servers calls the customer, who then opens them at the connector (#181). remotehub
should ask by itself, without the channel becoming a way into the customer's network.

## Decision

- **A user who may `connect` asks** for a device, or for a folder's devices behind its connector, for 15
  minutes to a day, with a reason. The targets, each a name, an address and a port, are fixed when asking.
  A request waits a day, then counts as expired. The requester may withdraw it before.
- **The connector asks, remotehub answers.** remotehub answers each state report with the pending
  requests. The connector reports every 10 seconds instead of every minute. remotehub still has no socket
  to write into while access is closed.
- **The answer is data, never a command.** Nothing in it changes the access. The connector takes it only
  whole: at most 64 KiB, 20 requests and 20 targets each, no unknown fields, host names and ports valid,
  text bounded and free of control, bidirectional and invisible characters. Otherwise it drops the answer
  and logs why. remotehub writes names that way, so one odd device name cannot block the channel.
- **Only a signed-in user of the connector approves or refuses**, in its web interface; there is no command
  for it. The page shows each target's address and port beside remotehub's name, as escaped text, and marks
  addresses that are not on the customer's list (ADR 0012). Its CSP allows only its own script.
- **An approval opens exactly its targets, each on its port, for the time asked.** It is stored beside the
  customer's list in `access.json`, can be closed early, and leaves the file when it closes. The network and
  the list do not change.
- **The answer goes back with the reports**, with the user who gave it, until remotehub no longer lists the
  request. remotehub records the first answer to a pending request of that connector and ignores others.
- **Both sides record it.** The connector's journal has the request, the answer and its user. remotehub's
  audit log has the request, the withdrawal and the answer.

## Consequences

- A compromised remotehub can show the customer requests, but opens nothing a person at the connector did
  not approve, and the approval names the address.
- Reports come six times as often. Each is one small HTTPS request.
- The customer learns of a request only on the connector's page. Notifying them, e.g. by mail, is left for
  later.
