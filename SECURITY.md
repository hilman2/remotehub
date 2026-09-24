# Security policy

remotehub holds the keys to other systems, so we take reports seriously.

## Reporting a vulnerability

Please **do not open a public issue**. Report privately through GitHub:
[Report a vulnerability](https://github.com/hilman2/remotehub/security/advisories/new).

Include what you found, how to reproduce it and which version you tested. You will get an answer within
a few days; fixes are released as soon as possible and credited in the advisory unless you prefer otherwise.

## Supported versions

remotehub is in early development. Only the latest release receives security fixes.

## Scope notes

- Until a second factor for directory sign-ins exists (milestone M4), remotehub is meant for internal
  networks only. Exposure to the internet is not a supported setup yet.
- To let people connect with their own directory account, the sign-in password is kept for the session,
  encrypted with a key that exists only in the user's cookie. Set `REMOTEHUB_OWN_ACCOUNT_CONNECTIONS=false`
  to switch this off.
- The vault must decrypt stored credentials on the server to inject them into sessions. Whoever controls
  the server and its key file can decrypt them; protect both accordingly.
