# ADR 0007: Web interfaces open in an isolated browser on the server

- Status: accepted
- Date: 2026-09-24

## Context

Appliances are administered through web interfaces (iDRAC, iLO, switches, firewalls). ADR 0003 reserved
"HTTPS" as a protocol: a Chromium on the server, shown through guacd, with the login filled in on the
server. #16 asked for it. The questions were how remotehub starts and controls browsers, how a browser is
kept to its one device, how its certificate is trusted, and how Chromium is sandboxed in a container.

A spike against the lab's web target (Debian trixie, Chromium from Debian, TigerVNC's Xvnc) showed:

- Chromium drawn into Xvnc needs no window manager with `--kiosk` and a window size.
- `--remote-debugging-pipe` gives the DevTools Protocol on file descriptors 3 and 4, without a TCP port.
  Focusing the fields with `Runtime.evaluate`, typing with `Input.insertText` and pressing Enter with
  `Input.dispatchKeyEvent` signs in to the lab's form.
- `--ignore-certificate-errors-spki-list` accepts the self-signed certificate with its key's hash and
  shows "Privacy error" with any other hash.
- In kiosk mode, F12 and Ctrl+Shift+I show "DevTools not allowed on this page", also without any policy;
  without kiosk mode they open the developer tools. The policy `DeveloperToolsAvailability: 2` also makes
  `Target.attachToTarget` on the pipe fail with "Not allowed".
- Chromium's sandbox needs user namespaces. Docker's default seccomp profile forbids them, and Chromium
  then refuses to start. With `seccomp:unconfined` it starts with its sandbox.

## Decision

- **A browser service** (`deploy/browser`, image `remotehub-browser`) runs next to guacd, on the internal
  network only. Its agent (`crates/browser`) listens on port 4823. remotehub sends one JSON line: URL,
  display size, certificate key hash and the credentials. The agent starts an Xvnc display with a random
  VNC password and a Chromium with a fresh profile, and answers with the display's port and password.
  remotehub then opens that display through guacd like a VNC device.
- **The TCP connection is the session.** When remotehub closes it, the agent ends Chromium and Xvnc and
  deletes the profile. When Chromium or Xvnc ends, the agent closes the connection. A remotehub restart
  leaves nothing behind.
- **One device per browser.** Chromium reaches the network only through a proxy of its own in the agent,
  which allows `CONNECT` to the device's host and port and refuses everything else. Other devices, the
  internet and the internal network stay out of reach, also through links and redirects. WebRTC may not
  use UDP outside the proxy. Only HTTPS devices.
- **Trust on first use** as for RDP: remotehub reads the device's certificate itself, pins its fingerprint
  on the first connection and refuses a changed one. Chromium gets the hash of that certificate's public
  key and accepts no other self-signed or wrongly named certificate.
- **The login is filled in by the agent** through the DevTools pipe: the first visible password field of
  the device's page, the text field before it, then Enter. The credentials never reach the admin's
  browser. Kiosk mode shows no address bar and keeps the developer tools closed. A managed policy turns
  off the password manager, downloads, extensions and `file://`, `view-source:` and `javascript:` URLs.
- **Chromium keeps its sandbox.** The browser container runs with `seccomp:unconfined` so that Chromium can
  create its user namespaces. It stays non-root, read-only, with `no-new-privileges` and no capabilities.

## Consequences

- A renderer exploit must also break Chromium's sandbox to reach other sessions. Without the sandbox
  (`--no-sandbox` under Docker's default seccomp profile) it would reach every session of the container,
  including other users' signed-in appliances. The price is that the agent and Chromium's browser process
  see the kernel's full system call interface.
- All sessions of a browser container share one Unix user. Installations that separate tenants run one
  browser service per tenant.
- A device's page can show the password if it has a "show password" button. The login is filled in and
  submitted at once, so the field is usually gone by the time the admin sees the page.
- The image carries Chromium from Debian and gets its security updates with image rebuilds only.
- Sign-in forms that are not a password field with a text field before it in the page itself (two-step
  logins, forms in frames) are not filled in; the admin sees the page and can sign in with personal
  credentials.
- Measured on the lab's sign-in page: about 170 MB and 105 processes per session. The ops package allows
  eight sessions in 3 GB.
