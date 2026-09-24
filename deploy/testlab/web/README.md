# Web target of the test lab — for tests only

An appliance's sign-in page, for HTTPS devices and the browser service (`deploy/browser`).

- **HTTPS** on 443: a form with user name and password; `tester` / `Tester-Passw0rd!` signs in. The
  certificate is self-signed (CN `web-target`) and new with every image build, so tests read it from the
  target instead of naming it.
- The page is blue (`#1e5b8c`), green (`#2e7d32`) once signed in and red (`#b3261e`) after a wrong password,
  for tests that look at the picture.
- The page also loads an image from port 8443, which stands for another device: the browser service must
  never reach it.
- **HTTP** on 8080, `GET /last`: the last sign-in as JSON, `{"username", "ok", "count", "escapes"}`, where
  `escapes` counts requests that reached port 8443. Never the password.

The password is public on purpose: it opens nothing but this container.
