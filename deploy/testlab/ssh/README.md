# SSH target of the test lab — for tests only

- User `tester`, password `Tester-Passw0rd!`, or the key pair `tester_ed25519` in this directory.
- Password, public-key and keyboard-interactive authentication are enabled.
- Host keys are generated when the image is built and stay the same for the image's lifetime.

The private key is public on purpose: it opens nothing but this container. Never use it anywhere else.
