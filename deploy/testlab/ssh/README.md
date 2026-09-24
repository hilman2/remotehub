# SSH target of the test lab — for tests only

- User `tester`, password `Tester-Passw0rd!`.
- Key `tester_ed25519` (no passphrase) and key `tester_ed25519_passphrase` (passphrase `Key-Passw0rd!`)
  are in `authorized_keys`.
- Key `tester_ed25519_cert` is **not** in `authorized_keys`; it only gets in with its certificate
  `tester_ed25519_cert-cert.pub`, signed by the test user CA `user_ca.pub` (principal `tester`, valid until
  2046), which the target trusts (`TrustedUserCAKeys`). The CA's private key was deleted after signing.
- Password, public-key, certificate and keyboard-interactive authentication are enabled.
- Host keys are generated when the image is built and stay the same for the image's lifetime.

The private keys are public on purpose: they open nothing but this container. Never use them anywhere else.
