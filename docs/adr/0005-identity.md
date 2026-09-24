# ADR 0005: Identity — Active Directory first, one provider trait

- Status: accepted
- Date: 2026-09-24

## Context

remotehub's users are administrators in Windows-centric organisations. Many still run on-premises Active
Directory; others use Entra ID. The product owner wants AD via LDAP first.

Signing in with LDAP means the server sees the user's password. That enables connecting "as yourself" to
targets (the password can be passed on), but LDAP offers no modern second factor. Signing in with OIDC
(Entra ID) gives MFA and Conditional Access, but the password never reaches remotehub.

## Decision

- **One trait `IdentityProvider`** in `crates/directory`; LDAP (M1) and OIDC (M4) are implementations.
- **LDAP sign-in:** LDAPS or StartTLS (rustls). The password is verified by binding as the user. Attributes
  and all nested groups are read through a service account from `tokenGroups`, with
  `LDAP_MATCHING_RULE_IN_CHAIN` as fallback.
- **Immutable identifiers:** users and groups are stored by `objectSid`/`objectGUID` (Entra: object IDs),
  never by name.
- **Connecting with the own AD account:** at sign-in the password is encrypted with a random per-session key.
  Only the ciphertext is kept on the server; the key lives only in the session cookie. Without the user's
  cookie the ciphertext is worthless; it expires with the session. The feature can be disabled globally.
- **Break-glass accounts:** local accounts (argon2id + mandatory TOTP), created only via the server's CLI,
  work when the directory is down. Every use is audited prominently.
- **Entra ID (M4):** authorization code flow with PKCE; app roles for coarse roles, security groups for
  permissions; on group overage the groups are fetched from Microsoft Graph (`transitiveMemberOf`) and
  cached.
- **Second factor for LDAP sign-ins (M4):** TOTP and WebAuthn. Until then remotehub is meant for internal
  networks only (README, SECURITY.md).

## Consequences

- Holding sign-in passwords, even encrypted and bound to the cookie, is sensitive; it is opt-out per
  instance and never persisted beyond the session.
- Entra users cannot connect "as themselves" to AD targets without a password; that needs passwordless RDP
  (backlog: smart-card emulation) or the *ask* mode.
- Permissions survive renames of users and groups because they reference SIDs and object IDs.
