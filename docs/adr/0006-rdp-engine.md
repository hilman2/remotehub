# ADR 0006: RDP stays on guacd; IronRDP is the way to Kerberos and smart cards

- Status: accepted
- Date: 2026-09-24

## Context

RDP runs in guacd 1.6 with FreeRDP 3 (ADR 0003). An own engine on [IronRDP](https://github.com/Devolutions/IronRDP)
behind the same `ProtocolEngine` trait would remove the C process, and could do what guacd 1.6 cannot. #23
asked for an evaluation. It was done with a spike (`spikes/ironrdp`, not part of the build) against the test
lab's desktop target (xrdp, TLS without NLA), with IronRDP 0.17 and sspi 0.21.

### Measured

| | Result |
|---|---|
| Connection (TCP, X.224, TLS, capabilities) | 107–118 ms, six runs |
| Lab desktop drawn (`#1e5b8c` at two points) | after 1.12–1.20 s, which is xrdp starting the session |
| Data until the desktop stood still | 6 PDUs, 20 786 bytes, 1024×768 |
| Without autologon | xrdp's own sign-in screen: without NLA, credentials only pass with autologon |
| Wrong password (xrdp, no NLA) | no error; the session stays black |
| Spike binary, release | 15 MB, with reqwest and aws-lc from sspi's network client |

The lab has no Windows target: CredSSP, Kerberos, the graphics pipeline and every codec below were not
measured. guacd's path in the lab reaches its first picture from xrdp after 0.11–0.16 s
(`rdp_shows_the_desktop_with_a_pinned_certificate`); that is the first frame, not the finished desktop, so the
two numbers do not compare.

### Read in the crates

- **Authentication (sspi):** NTLM, Kerberos (also through a KDC proxy), PKINIT and smart-card emulation
  (feature `scard`). guacd 1.6 authenticates with NTLM only; Kerberos is planned for guacd 1.7
  (GUACAMOLE-2057).
- **Graphics:** `ironrdp-graphics` has RemoteFX, progressive RemoteFX, ClearCodec, planar and interleaved
  bitmaps and ZGFX; NSCodec and the graphics pipeline are separate crates (`ironrdp-nscodec`,
  `ironrdp-egfx`). Whether the pipeline decodes H.264/AVC was not checked.
- **Channels:** clipboard, display control, audio output, device redirection (`rdpdr`) and the RD Gateway
  protocol (`mstsgu`) exist as separate crates.
- **Maintenance:** Devolutions develops IronRDP; all crates are 0.x, so every minor release may change
  their API.

### What an engine would take

IronRDP decodes into a framebuffer. The browser keeps the Guacamole client, so remotehub would do what
guacd does now: track changed regions, encode them as PNG, JPEG or WebP, send them as Guacamole `img`
streams with layers and cursor, and translate Guacamole input back into RDP. Add clipboard, display
resizing, keyboard layouts, certificate pinning and, for M5, recording. This is the bulk of the 2–4
person-months estimated in #23, and it moves image encoding into the Rust server, whose CPU budget it then
shares.

## Decision

- **RDP stays on guacd** for now. It works with the features remotehub has, and the lab cannot prove an
  own engine against Windows.
- **IronRDP becomes the engine for what guacd cannot do**, when that is needed: Kerberos for members of
  *Protected Users* and domains without NTLM, and smart-card logon with certificates from a remotehub CA
  (#21). It would serve only devices that need it, behind the same trait, so the rest stays on guacd.
- Before that work starts, the test lab needs a Windows target with NLA, or the measurements above stay the
  only ones.

## Consequences

- #21 depends on this engine, not on guacd, unless guacd 1.7 brings Kerberos first.
- The spike stays in `spikes/ironrdp` to repeat the measurements; it accepts any certificate and is never
  built into remotehub.
- Recording (M5) stays guacd's `.guac` format; an IronRDP engine would have to write the same format.
