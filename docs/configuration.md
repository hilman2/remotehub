# Configuration reference

remotehub reads its settings from environment variables. Every variable `REMOTEHUB_X` can also be given as
`REMOTEHUB_X_FILE`, the path of a file holding the value; the file wins if both are set. An empty value
counts as unset. The ops package (`deploy/ops`) sets them in `compose.yml` from `.env` and from the files
in `secrets/`; see [Installing remotehub](install.md).

## Server

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_PUBLIC_URL` | required | The address people open, e.g. `https://remotehub.example.com`. Requests that change state must come from this origin. The session cookie is `Secure`, so it must be `https` except on `localhost`. |
| `REMOTEHUB_LISTEN` | `0.0.0.0:8080` | Address and port the server binds to. |
| `REMOTEHUB_TRUSTED_PROXIES` | – | Reverse proxies, as addresses or ranges (`10.0.0.0/8`), separated by commas. A request from one of them names its client in `X-Forwarded-For`: the right-most address there that is no trusted proxy counts, for the audit log and the limit on failed sign-ins. From anywhere else the header is ignored. The ops package sets the gateway of its network. |
| `REMOTEHUB_WEB_DIR` | set in the image | Directory of the built web UI. Without it the server serves only the API. |
| `REMOTEHUB_LOG_FORMAT` | `text` | `text` or `json` (one JSON object per line). |
| `RUST_LOG` | `info` | Log levels, e.g. `info,remotehub_server=debug`. |

## Database

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_DATABASE_URL` | required | PostgreSQL URL, e.g. `postgres://remotehub@db:5432/remotehub`. Migrations run at startup. |
| `REMOTEHUB_DATABASE_PASSWORD` | – | Password of the database user. Replaces a password in the URL. |

## Vault

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_MASTER_KEY_FILE` | required | Path of the master key file. Only as a path: the key is never read from the environment. The server warns if other users may read the file. |

The file holds one line per key version; `remotehub generate-key --version N` prints a new line. Appending
the next version and restarting makes it the current key.

## Sessions

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_SESSION_IDLE_MINUTES` | `30` | A session ends after this long without a request. At most the maximum below. |
| `REMOTEHUB_SESSION_MAX_HOURS` | `12` | A session ends this long after sign-in. |

## Directory (Active Directory over LDAP)

Without `REMOTEHUB_LDAP_URL`, only break-glass accounts can sign in.

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_LDAP_URL` | – | `ldaps://host[:port]`, or `ldap://host[:port]` with StartTLS. |
| `REMOTEHUB_LDAP_STARTTLS` | `false` | Upgrade an `ldap://` connection with StartTLS. `ldap://` without it is refused. |
| `REMOTEHUB_LDAP_CA_FILE` | system CAs | PEM file with the CA of the directory's certificate. |
| `REMOTEHUB_LDAP_BIND_DN` | required with a URL | Service account for searches, e.g. `svc-remotehub@example.com`. |
| `REMOTEHUB_LDAP_BIND_PASSWORD` | required with a URL | Its password. |
| `REMOTEHUB_LDAP_BASE_DN` | required with a URL | Where users are searched, e.g. `DC=example,DC=com`. |
| `REMOTEHUB_LDAP_USER_FILTER` | – | An LDAP filter that users must match as well, e.g. `(memberOf=CN=remotehub users,OU=Groups,DC=example,DC=com)`. |
| `REMOTEHUB_LDAP_TIMEOUT_SECONDS` | `10` | Time limit for each directory request. |

## Local accounts (Ory Kratos)

Without `REMOTEHUB_KRATOS_URL`, only directory and break-glass accounts can sign in. The ops package runs
Kratos and sets both variables; its own settings are in `kratos/kratos.yml` and `secrets/kratos.yml`.

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_KRATOS_URL` | – | Kratos' public API, e.g. `http://kratos:4433`. Browsers reach it through remotehub under `/api/auth/`. |
| `REMOTEHUB_KRATOS_ADMIN_URL` | required with a URL | Kratos' admin API, e.g. `http://kratos:4434`. |

## Administration

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_ADMIN_GROUPS` | – | Groups whose members administer remotehub, separated by commas or semicolons: SIDs, or group names that are looked up in the directory at startup. |
| `REMOTEHUB_ADMIN_ACCOUNTS` | – | Local accounts that administer remotehub: their e-mail addresses, separated by commas or semicolons. |

## Connections

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_OWN_ACCOUNT_CONNECTIONS` | `true` | Keep the sign-in password, encrypted with a key held only in the user's cookie, so devices can be opened with the user's own directory account. |
| `REMOTEHUB_GUACD` | `guacd:4822` | guacd for RDP and VNC, as `host:port`. |
| `REMOTEHUB_BROWSER` | `browser:4823` | The browser service for web interfaces (HTTPS devices), as `host:port`. guacd must reach the same host. |
| `REMOTEHUB_SSH_CA_KEY_FILE` | – | Path of the SSH CA's private key (Ed25519, OpenSSH format, from `remotehub generate-ssh-ca`). Only as a path. With it, SSH devices can sign in with a certificate, and `/api/ssh-ca.pub` serves the public key. |
| `REMOTEHUB_RDP_KEYBOARD_LAYOUT` | `en-us-qwerty` | Keyboard layout of RDP sessions for devices without their own. |

Keyboard layouts: `cs-cz-qwertz`, `da-dk-qwerty`, `de-ch-qwertz`, `de-de-qwertz`, `en-gb-qwerty`,
`en-us-qwerty`, `es-es-qwerty`, `es-latam-qwerty`, `fr-be-azerty`, `fr-ca-qwerty`, `fr-ch-qwertz`,
`fr-fr-azerty`, `hu-hu-qwertz`, `it-it-qwerty`, `ja-jp-qwerty`, `no-no-qwerty`, `pl-pl-qwerty`,
`pt-br-qwerty`, `pt-pt-qwerty`, `ro-ro-qwerty`, `sv-se-qwerty`, `tr-tr-qwerty`, and `failsafe`, which sends
characters instead of keys.

## Browser service

Settings of the `remotehub-browser` container, which opens web interfaces in Chromium.

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_BROWSER_SESSIONS` | `8` | Web interfaces open at once, at most 99. Session n uses VNC port 5900 + n. |
| `REMOTEHUB_BROWSER_LISTEN` | `0.0.0.0:4823` | Address and port for remotehub. |
| `REMOTEHUB_BROWSER_MEMORY` | `3g` | Ops package only: the container's memory limit. |

## Site connector

Settings of `remotehub connector`, which runs in another network and reaches devices there for remotehub.

| Variable | Default | Meaning |
|---|---|---|
| `REMOTEHUB_URL` | required | remotehub's public address, e.g. `https://remotehub.example.com`. |
| `REMOTEHUB_CONNECTOR_TOKEN` | required | The token shown when the connector was created. Better as `REMOTEHUB_CONNECTOR_TOKEN_FILE`. |
| `REMOTEHUB_CONNECTOR_CA_FILE` | system CAs | PEM file with the CA of remotehub's certificate. |
| `REMOTEHUB_CONNECTOR_ALLOW` | – | Address ranges the connector may connect to, separated by commas, e.g. `10.20.0.0/16`. Without it, every address. |
| `REMOTEHUB_LOG_FORMAT` | `text` | `text` or `json`. |

## Command line

`remotehub` alone runs the server. In the ops package, run the other commands with
`docker compose exec remotehub remotehub <command>`. They answer in the language of `LC_ALL`, `LC_MESSAGES`
or `LANG` (English or German, English otherwise), e.g. `docker compose exec -e LANG=de remotehub remotehub
verify-audit`.

| Command | Does |
|---|---|
| `generate-key [--version N]` | Prints a new line for the master key file. |
| `generate-ssh-ca` | Prints a new SSH CA key for `REMOTEHUB_SSH_CA_KEY_FILE`. |
| `break-glass create NAME` | Creates a local emergency account and prints its password and TOTP secret once. |
| `break-glass reset NAME` | Replaces both; the account's open sessions end. |
| `break-glass delete NAME` | Deletes the account. |
| `break-glass list` | Lists the accounts. |
| `account invite EMAIL [--name NAME]` | Creates a local account and prints the link and one-time code, valid for 48 hours, with which its owner sets a password and an authenticator app. |
| `verify-audit` | Checks the audit log's hash chain; exits with 1 if it is broken. |
| `connector` | Runs as a site connector (see above) instead of the server. |
| `healthcheck` | Asks the running server for `/api/health`; exits with 1 unless it answers 200. |
