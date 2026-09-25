# OpenID Connect provider of the test lab — for tests only

[Dex](https://dexidp.io/) stands in for Entra ID, Google or GitHub (#109). Kratos knows it as the provider
`lab`, "Sign in with Lab" on the sign-in page.

- Issuer `http://oidc:5556/dex`, on the compose network only. The end-to-end tests reach it because their
  browser shares the network of the web container; a browser on the host does not.
- Client `remotehub` with secret `lab-oidc-secret`, for the redirect URIs of development (port 5180) and the
  local CI (port 8080).
- People: `linda@remotehub.test` and `stranger@remotehub.test`, both with password `password`. The tests link
  linda to a local account and keep stranger unknown.

Nothing is stored: every start forgets what happened before. The secret and the passwords open nothing but
this container.
