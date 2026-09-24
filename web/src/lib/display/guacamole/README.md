# guacamole-common-js (vendored)

The JavaScript client of Apache Guacamole 1.6.0, which draws RDP and VNC sessions that guacd renders on
the server (ADR 0003). Apache License 2.0 — see `LICENSE` and `NOTICE`.

- Source: `guacamole-client-1.6.0.tar.gz` from https://archive.apache.org/dist/guacamole/1.6.0/source/,
  SHA-256 `81f9fd5a7b4377fb0ee295d0d4fec92e9667f2aafaa3d0ed8937f535deabdee4`.
- `guacamole-common.js` is `guacamole-common-js/src/main/webapp/common/license.js` followed by every file
  of `…/modules/` in byte order of the file names (as the upstream build concatenates them), and
  `export default Guacamole;` at the end. Nothing else is changed.
- `guacamole-common.d.ts` types the parts remotehub uses; extend it when using more.

To update: take the new release's source tarball, check its checksum against the `.sha256` file on
archive.apache.org, rebuild `guacamole-common.js` the same way, and keep guacd (`deploy/guacd`) on the
same version.
