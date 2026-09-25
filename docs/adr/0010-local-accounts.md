# ADR 0010: Local accounts through Ory Kratos

- Status: accepted
- Date: 2026-09-25

## Context

remotehub started with Active Directory as its only source of users (ADR 0005). Teams without a directory, a
web agency that manages its VPS from Macs for instance, could not use it at all.

Accounts of their own need more than a password table: password rules and a check against leaked
passwords, a second factor, recovery, invitations. Built by hand, each of these is a place for the most
dangerous mistakes. Keycloak does all of it, but is heavy (Java, around 1 GB of memory) and its pages only
partly take remotehub's look.

## Decision

- **Ory Kratos (Apache 2.0) keeps the local accounts.** It is headless: remotehub renders every page itself
  from Kratos' flows, so sign-in, recovery and account settings look like the rest of remotehub.
- **Browsers reach Kratos only through remotehub,** under `/api/auth/`. Only the self-service flows and the
  session check pass; cookies stay on remotehub's origin; the admin API stays on the internal network.
- **remotehub keeps its own session.** `POST /api/session/local` turns a Kratos session into a remotehub
  session. Kratos proves who someone is; `authorize()` still decides what they may do.
- **A second factor is required.** Kratos' open-source version cannot demand that an account has one, so
  remotehub does: an account without one is sent to set up an authenticator app before its first session.
- **Accounts come from invitations.** Registration is closed; `remotehub account invite` creates the
  account and a one-time code. The first administrator is invited this way and named in
  `REMOTEHUB_ADMIN_ACCOUNTS`.
- **Local accounts are named by their Kratos identity** (`local:<identity>` in grants), never by name, like
  directory users by their SID.
- **AD stays a way of its own** through LDAP, next to Kratos, so connecting with the own AD account keeps
  working. Break-glass accounts stay, for when the directory or Kratos is down.
- **Kratos sends no usage reports** (`SQA_OPT_OUT`).

## Consequences

- One more service to run: Kratos with its own database on the same PostgreSQL server, a few dozen MB of
  memory. The ops package runs it; `init.sh` makes its secrets.
- Kratos stores the keys of authenticator apps and the recovery codes in plain text in its database. Its
  database needs the same protection as remotehub's own, and both belong in every backup.
- Kratos does not slow down password guessing; remotehub counts failed sign-ins through its proxy per
  address.
- SAML, SCIM and CAPTCHA are in Kratos' enterprise edition only and are not used.
- Passkeys need WebAuthn ceremonies in remotehub's own pages; they come in a later step.
