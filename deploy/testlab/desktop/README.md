# Desktop target of the test lab — for tests only

- **RDP** on 3389 (xrdp): user `tester`, password `Tester-Passw0rd!`. TLS only (`security_layer=tls`) with the
  fixed certificate in `tls/` (CN `desktop-target`, valid until 2046, SHA-256 fingerprint
  `C1:E8:6D:13:4E:8D:B7:A5:D2:72:01:8F:93:8F:C4:44:EC:E4:C0:D5:97:C8:00:EF:25:24:BB:76:22:0B:DF:CD`), so
  certificate pinning can be tested. Each sign-in gets its own Xvnc session.
- **VNC** on 5900 (TigerVNC): password `Vnc-Pw1!` (VNC passwords have at most eight characters), no user
  name. One shared display, 1280×800.
- Both desktops show the background colour `#1e5b8c` and an xterm titled `remotehub lab RDP` or
  `remotehub lab VNC`, for tests that look at the picture.
- The clipboard answers: text that arrives on it is replaced with `echo:<text>` a moment later
  (`clipboard-echo`), so a test can check both directions of the clipboard.

The passwords and the private key are public on purpose: they open nothing but this container. Never use
them anywhere else.
