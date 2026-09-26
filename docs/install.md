# Installing remotehub

remotehub runs as a handful of containers: the server, PostgreSQL, Kratos for local accounts, guacd, the
engine for RDP and VNC, the browser service, which opens web interfaces of devices, and, on a host of its
own, Caddy for HTTPS. The ops package in [`deploy/ops`](../deploy/ops) starts them with Docker Compose.
Every setting is described in the [configuration reference](configuration.md).

You need a host with Debian 12 or 13 or Ubuntu 22.04 or 24.04 on x86_64, with 4 GB of memory and 20 GB of
disk, and a DNS name for remotehub. To let people sign in with Active Directory, you need a service account
there that may read users and groups.

## Install

On the host, as root:

```bash
curl -fsSL https://github.com/hilman2/remotehub/releases/latest/download/install.sh | sudo sh
```

It asks for the name people will open remotehub under, e.g. `remotehub.example.com`, and ends with a link
to the setup wizard (see [First sign-in](#first-sign-in)). On its way, it:

- installs Docker and its Compose plugin if they are missing;
- downloads the ops package of the release and checks it against the release's `SHA256SUMS`;
- puts it into `/opt/remotehub`, writes `.env` and creates the secrets (see [Secrets](#secrets));
- sets up HTTPS as the next two sections describe, and starts everything.

It logs to `/var/log/remotehub-install.log`. Running it again on an installed host changes nothing and shows
the link again, as long as setup is not done; it never upgrades (see [Upgrade](#upgrade)).

For an install without questions: `--domain NAME`, `--version X.Y.Z` for another release than the latest,
and `--mode caddy` or `--mode external` to choose what the next two sections describe.

A host that cannot reach GitHub installs from a mirror: copy `install.sh`, `remotehub-ops.tar.gz` and
`SHA256SUMS` of the release to a web server it reaches, then run
`curl -fsSL https://mirror.example.com/remotehub/install.sh | sudo sh -s -- --from https://mirror.example.com/remotehub`.
The images still come from `ghcr.io`.

## HTTPS on a host of its own

If nothing listens on ports 80 and 443, the installer turns on Caddy of the ops package
(`COMPOSE_PROFILES=caddy` in `.env`) and opens both ports in `ufw` or `firewalld` if one of them is active.
Caddy gets a certificate from Let's Encrypt if the internet reaches the host under its name. Otherwise, which
is the usual case for an internal tool, it signs one with a CA of its own, named after the host. Browsers
warn about that CA until the clients trust it. A certificate from that CA lasts 12 hours, and each renewal
asks Let's Encrypt first: a host the internet reaches later gets a Let's Encrypt certificate within hours,
without a change.

*Settings → Certificate* shows which certificate Caddy serves. With Caddy's own CA, it offers the CA's root
certificate for the clients, as `.crt` and as `.cer` for Windows, with its fingerprint and how to install it
on Windows through group policy, on macOS and on Linux. Anyone can fetch it from `https://HOST/ca.crt`.
Compare the fingerprint on the client with the one on the page.

A certificate of your own, from your company's CA or a public one, is uploaded on the same page as a PFX file
or as PEM files. remotehub refuses a certificate that does not cover the host, whose key does not belong to
it, or that is not valid for at least another day. Caddy serves it at once, and a banner warns 30 days
before it runs out. *Back to automatic* returns to Let's Encrypt or Caddy's own CA.

Caddy keeps its certificates and its CA's key in `caddy/data` (see [Back up and restore](#back-up-and-restore)).
A certificate of your own lives in the database, its key sealed like a password; remotehub writes it to
`caddy/remotehub` for Caddy every time it starts.

## Next to a reverse proxy of your own

If port 80 or 443 is taken, the installer attaches remotehub to the program on it and says what it did:

| On the host | What the installer does |
|---|---|
| Caddy | writes `/etc/caddy/remotehub.caddy`, adds an `import` of it to the Caddyfile, checks and reloads |
| nginx | writes `/etc/nginx/conf.d/remotehub.conf`, with the Let's Encrypt certificate of the name if there is one, otherwise a self-signed one; checks and reloads |
| Traefik in Docker, with its Docker provider | puts remotehub on Traefik's network with router labels, in `compose.override.yml` |

If the check fails, it puts the proxy's configuration back. For anything else, it prints what the proxy
needs. remotehub listens on a free port on `127.0.0.1` (`REMOTEHUB_PORT` in `.env`), speaks plain HTTP
there, and needs from the proxy:

- WebSockets passed on, which carry the terminal and the remote desktops;
- connections kept open for hours;
- the client named in `X-Forwarded-For`.

With nginx, for example:

```nginx
server {
    listen 443 ssl;
    server_name remotehub.example.com;
    ssl_certificate     /etc/ssl/remotehub.crt;
    ssl_certificate_key /etc/ssl/remotehub.key;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_read_timeout 12h;
        proxy_send_timeout 12h;
    }
}

map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}
```

remotehub refuses changes from pages under any origin other than `REMOTEHUB_PUBLIC_URL`, so people must open
exactly that address, not another name of the same host.

remotehub believes `X-Forwarded-For` only from Caddy of the package and from the gateway of its Docker
network, which is how a proxy on the host reaches it. The network uses `10.213.213.0/24`; guacd and the
browser service cannot reach devices in that range. If your network uses it, set another range in `.env`
before the first start, Caddy's address within it:

```
REMOTEHUB_SUBNET=10.99.99.0/24
REMOTEHUB_GATEWAY=10.99.99.1
REMOTEHUB_CADDY_ADDRESS=10.99.99.254
```

A proxy elsewhere than on this host goes into `REMOTEHUB_TRUSTED_PROXIES` in `compose.yml` instead.

## Secrets

The installer creates these files in `secrets/` once and never overwrites them:

| File | Holds | Owner, mode |
|---|---|---|
| `master_key` | The key that encrypts every stored credential. | 65532, 0400 |
| `db_password` | The database password, read by PostgreSQL and remotehub. | 65532:999, 0440 |
| `ssh_ca_key` | The key of remotehub's SSH certificate authority. | 65532, 0400 |
| `courier_token` | The token with which Kratos hands its mails to remotehub. | 65532, 0400 |
| `kratos.yml` | Database and secrets of Kratos, which keeps the local accounts, with the courier token. | 10000, 0400 |

remotehub runs as user 65532, PostgreSQL as 999 and Kratos as 10000, so the files belong to them; `secrets/`
itself is open to root only. Keep these owners and modes when you edit a file, e.g. with `sudo tee secrets/kratos.yml`.

Without `master_key`, the credentials in the database cannot be read by anyone. Keep a copy of it apart from
the database backups, or let the organisation recovery key hold it (see [Back up and restore](#back-up-and-restore)).
Keep a copy of `ssh_ca_key` too: a new one means every device must trust the new public key.

## First sign-in

A fresh installation waits for its setup. Open the link the installer printed. The wizard creates the first
administrator, a local account with a password and an authenticator app: remotehub asks for its code at
every sign-in. Then, if you want, it connects Active Directory (see
[Connect Active Directory](#connect-active-directory)) and gives a group of it the administrator role, and
it sets the mail server (see [Send mail](#send-mail)). Last, it creates a break-glass account for the day the
directory or Kratos is unreachable, and the organisation recovery key (see
[Recover personal vaults](#recover-personal-vaults)). Each comes on a page of its own to print; keep them
offline, in different places. The break-glass account signs in at `/sign-in/break-glass`.

The link works until the wizard has created the administrator. A new one, which the old one stops working
with:

```bash
cd /opt/remotehub
sudo docker compose exec remotehub remotehub setup-code
```

More break-glass accounts come from `remotehub break-glass create NAME`.

## Connect Active Directory

The wizard's directory step and *Settings → Directory* take the domain controller's address
(`ldaps://dc.example.com`, or `ldap://` with StartTLS under *More settings*), the service account, its
password and the base DN below which users are found. "Test connection" goes through the steps a sign-in
takes and names the one that fails: finding the server, connecting, TLS, signing in as the service account,
searching. Saving runs the same check and stores only what passes. The next sign-in uses it; nothing needs
a restart.

If the domain controller's certificate comes from a CA the system does not know, the check shows the
certificate the server presented, with its fingerprint. Compare the fingerprint, then trust it. Many
domain controllers send only their own certificate: trusted as it is, it counts until the controller gets a
new one. To cover renewals, paste your CA's certificate into *Trusted certificates* instead.

The password is stored encrypted like a vault entry and never shown again; leave the field empty to keep
it. A new address or base DN signs out every directory user, and so does removing the connection.

Under *Settings → Administrators*, give directory groups the administrator role.

## Local accounts

Local accounts live in Ory Kratos, which the ops package runs next to remotehub. People reach it only
through remotehub. Its settings are in `kratos/kratos.yml`; database and secrets in `secrets/kratos.yml`.

- **Inviting and managing:** administrators invite local accounts under *Users*. With a mail server (see
  [Send mail](#send-mail)), the link and the code go out by mail; the dialog shows them too, to hand over
  by hand. The same page blocks anyone, local or from the directory, ends their sessions, and deletes
  local accounts. Everything done there is in the audit log.
- **Groups:** without a directory, put local accounts into groups under *Users* and grant access to the
  groups. Directory users and groups can be members too.
- **Forgotten passwords:** with a mail server, "Forgot the password?" on the sign-in page mails a code, in
  the language of the browser. Without one, the page says to ask an administrator, who gives the person a
  *New sign-in code* under *Users*.
- **Second factor:** every local account sets up an authenticator app before its first session, and can
  create recovery codes under *My account*. A lost authenticator app takes a *New sign-in code* too: it
  removes the second factor, and the person sets up a new one with the code.
- **Passkeys:** under *My account*, a passkey (Touch ID, Face ID, Windows Hello, a security key) signs in
  instead of the password, and a security key can stand in for the authenticator app's code. Passkeys
  belong to `REMOTEHUB_HOST` in `.env`: changing it later makes every passkey useless.
- **Backups:** Kratos' database sits next to remotehub's; see [Back up and restore](#back-up-and-restore).

## Send mail

remotehub sends invitations, new sign-in codes and the codes for a forgotten password itself; Kratos hands
its mails to remotehub. Set the mail server in the setup wizard or under *Settings → Mail*: host, port,
encryption, user and password, and the sender. "Send test mail" sends one with the form as it is and
names what failed: the address, the connection, TLS, the sign-in, or the server's refusal, with its answer.

- **Encryption:** TLS (usually port 465) or STARTTLS (587). *None* is for a relay inside your network,
  usually on port 25; it takes no user name and no password, since they would travel unencrypted.
- **Your own CA:** if the server's certificate comes from one, paste the CA's certificate under *More
  settings*.
- **The password** is stored encrypted like a vault entry and never shown again. Leave the field empty to
  keep it; it stays only as long as the user name does.

## Sign in with Entra ID, Google or GitHub

Local accounts can sign in through an OpenID Connect provider instead of with their password. remotehub still
asks for the code of the authenticator app.

1. Register remotehub as an application with the provider. Its redirect URI is
   `https://remotehub.example.com/api/auth/self-service/methods/oidc/callback/<id>`, with the `id` you give
   the provider in the next step.
2. Add the provider to `secrets/kratos.yml`:

   ```yaml
   selfservice:
     methods:
       oidc:
         enabled: true
         config:
           providers:
             - id: entra
               label: Microsoft
               provider: microsoft
               microsoft_tenant: 00000000-0000-0000-0000-000000000000
               client_id: 00000000-0000-0000-0000-000000000000
               client_secret: the-client-secret
               mapper_url: file:///etc/config/kratos/oidc.jsonnet
               scope: [openid, email, profile]
   ```

   For Google, use `provider: google`; for GitHub, `provider: github` and `scope: [user:email]`. Other
   providers are in [Kratos' documentation](https://www.ory.com/docs/kratos/social-signin/overview).
3. `sudo docker compose up -d kratos`. The sign-in page now offers "Sign in with Microsoft".

Each person links their account once: they sign in with the password, then use *My account → Link
Microsoft*. Someone whose account at the provider is linked to nobody is turned away.

To let the provider's people in without an invitation, add this to `secrets/kratos.yml`:

```yaml
selfservice:
  flows:
    registration:
      enabled: true
```

Anyone the provider signs in then gets an account and sets up the authenticator app. Do this only with a
provider that knows nobody but your organisation's people, such as Entra ID with your own tenant: with Google or
GitHub, anyone with an account there could join. Nobody can register with a password, with or without this.

## Second factor for directory accounts

Directory accounts sign in with their password only, unless they set up an authenticator app or a security
key under *My account*. A passkey (Touch ID, Face ID, Windows Hello) counts as a security key; either one
does as the second factor. To make it a condition, add users or groups under *Settings → Second factor for directory
accounts*: they set up the app at their next sign-in. After a lost phone, an administrator removes the
app under *Users*, and the person sets up a new one.

## Recover personal vaults

A personal vault is encrypted in its owner's browser. Without the organisation recovery key, it is lost
when its owner forgets the passphrase and the recovery key, or leaves.

1. Under *Vault recovery*, an administrator creates the recovery key. The browser downloads it as a file,
   sealed with a passphrase, and shows it once as text. Print the text and keep it in a safe; keep the file
   and its passphrase apart.
2. Give someone the role *Security officer* under *Users*. A recovery needs their approval, and they cannot
   approve one they asked for themselves.
3. Every vault is wrapped for the key at its owner's next unlock. *Vault recovery* shows which are.

To recover a vault, an administrator asks for it there, with a reason. Once a security officer approved,
the same administrator carries it out within a day, with the key file or the printed text:

- **Forgotten passphrase:** remotehub shows a one-time recovery key for the owner. With it, the owner
  unlocks the vault and chooses a new passphrase.
- **Hand-over:** the entries become credentials in a shared folder.

The private key never reaches the server. Every step is in the audit log. To replace the key, create a new
one; vaults move to it at their next unlock, and the old one can be deleted once no vault needs it.

## Sign in to SSH devices without stored passwords

A device set to sign in with "A certificate from remotehub" needs no credential. At every connection
remotehub signs a fresh key for the user's name, valid for five minutes. The device accepts it once its
sshd trusts remotehub's certificate authority:

```bash
curl -fsS https://remotehub.example.com/api/ssh-ca.pub | sudo tee /etc/ssh/remotehub_ca.pub
echo 'TrustedUserCAKeys /etc/ssh/remotehub_ca.pub' | sudo tee /etc/ssh/sshd_config.d/remotehub.conf
sudo systemctl reload ssh
```

The certificate's principal is the remotehub user name, e.g. `alice`, and the device needs an account of that
name. The target's log names the key as `remotehub <user> device <id>`.

## Sign in with LAPS passwords

A device set to sign in with "The local administrator from LAPS" uses the password that Windows LAPS or legacy
Microsoft LAPS keeps on the device's computer account. remotehub reads it at every connection with its
service account and stores it nowhere. It finds the computer account by the device's host name, so the host
must be the computer's DNS name or its first label, not an IP address.

Allow the service account to read the passwords of the computers in question, for Windows LAPS:

```powershell
Set-LapsADReadPasswordPermission -Identity "OU=Servers,DC=example,DC=com" -AllowedPrincipals "svc-remotehub"
```

and for legacy LAPS `Set-AdmPwdReadPasswordPermission` likewise. remotehub reads the plain-text
`msLAPS-Password` and `ms-Mcs-AdmPwd`; encrypted Windows LAPS passwords and Entra LAPS are not supported yet.

## Open web interfaces of devices

A device with the protocol "Web interface (HTTPS)" opens in a Chromium of the browser service, which reaches
that one device and nothing else. remotehub fills in its sign-in form, so the password never reaches the
admin's browser. It pins the device's certificate at the first connection, as for RDP.

The browser service runs with `seccomp:unconfined`, which Chromium's sandbox needs; `compose.yml` sets it.
Each open web interface takes a Chromium: on the lab's simple sign-in page about 170 MB and 105 processes.
Heavier interfaces need more. For more than eight at once, raise the limits in `.env`:

```
REMOTEHUB_BROWSER_SESSIONS=16
REMOTEHUB_BROWSER_MEMORY=6g
```

All sessions of the service run as one Unix user. Why, and what that means: [ADR 0007](adr/0007-isolated-browser.md).

## Reach devices in other networks

A site connector runs in a network that remotehub cannot reach and opens the way from there. It needs outbound
HTTPS to `REMOTEHUB_PUBLIC_URL` and nothing inbound. It is the image `ghcr.io/hilman2/remotehub-connector`.

The customer decides when remotehub may enter, and where. Access starts closed: the connector does not connect
to remotehub until someone opens it, for some hours, until a point in time, or without end. It opens the whole
network or only single devices and groups the customer lists. Closing, or the time running out, ends running
connections at once. remotehub can ask for access, and a person at the customer approves or refuses. The
connector keeps its own log of every change and every connection. Why the switch sits at the connector:
[ADR 0011](adr/0011-customer-controls-access.md), [ADR 0012](adr/0012-access-per-device.md) and
[ADR 0013](adr/0013-remotehub-asks-the-customer.md).

### Start the connector on Linux

Create the connector under *Connectors* in remotehub. Its token is shown once. On a Linux host in that network,
put the token into a file and start the connector of your remotehub's release:

```bash
version=0.3.0 # your remotehub's release: REMOTEHUB_VERSION in its .env
sudo install -d -m 700 /opt/remotehub-connector
sudo sh -c 'cat > /opt/remotehub-connector/token' # paste the token, then Ctrl+D
sudo chown 65532 /opt/remotehub-connector/token && sudo chmod 400 /opt/remotehub-connector/token
sudo docker run -d --name remotehub-connector --restart unless-stopped --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  -v /opt/remotehub-connector/token:/run/secrets/token:ro \
  -v remotehub-connector-data:/var/lib/remotehub-connector \
  -p 127.0.0.1:8480:8480 \
  -e REMOTEHUB_URL=https://remotehub.example.com \
  -e REMOTEHUB_CONNECTOR_TOKEN_FILE=/run/secrets/token \
  "ghcr.io/hilman2/remotehub-connector:${version}"
```

The volume `remotehub-connector-data` holds the access state, the log, the users of the web interface and its
certificate. Keep it when you replace the container.

The connectors page in remotehub shows the connector as *closed by the customer*. Choose it under *Reached
through* on the customer's folder: every device in it and below it takes it, unless the device says *Directly*
or names another connector. A single device can name it the same way. To keep the connector away from parts of
its network, list the ranges it may reach in `REMOTEHUB_CONNECTOR_ALLOW`.

### Start the connector on a Windows server

Each release has `remotehub-connector.exe`, a Windows service in one file. It is not signed yet. Download it
with PowerShell, which marks nothing as coming from the internet, and compare its hash with the release's
`SHA256SUMS`:

```powershell
$version = "0.3.0"  # your remotehub's release
$base = "https://github.com/hilman2/remotehub/releases/download/v$version"
Invoke-WebRequest "$base/remotehub-connector.exe" -OutFile remotehub-connector.exe
Invoke-WebRequest "$base/SHA256SUMS" -OutFile SHA256SUMS
(Get-FileHash remotehub-connector.exe -Algorithm SHA256).Hash
Select-String "remotehub-connector.exe" SHA256SUMS
```

Both hashes must match (`Get-FileHash` writes it in capitals). Then, in a PowerShell as administrator, install
the service and paste the token when asked. It never goes on the command line:

```powershell
.\remotehub-connector.exe install --url https://remotehub.example.com
```

`--allow 10.20.0.0/16` limits the ranges it may reach. The installer puts the program into
`C:\Program Files\remotehub-connector`, and the token, the settings (`connector.conf`), the access state, the
log and the users into `C:\ProgramData\remotehub-connector`, which only SYSTEM, the Administrators and the
service's account `LocalService` can read. The service `remotehub-connector` starts with Windows and restarts
after a failure. Its own log is `connector.log` in the data directory.

The web interface is at `https://localhost:8480` on the server. To reach it from the network, set
`REMOTEHUB_CONNECTOR_LISTEN=0.0.0.0:8480` in `connector.conf`, allow the port in the Windows firewall, and
restart the service (`Restart-Service remotehub-connector`).

`.\remotehub-connector.exe update`, run as administrator from the new release's file, replaces the program and
restarts the service. `uninstall` removes the service and the program and keeps the data; `uninstall --purge`
removes that too. Run both from a copy outside `C:\Program Files\remotehub-connector`.

### Open and close access

The connector's web interface is at `https://localhost:8480` on its host. `-p 127.0.0.1:8480:8480` keeps it
there; `-p 8480:8480` makes it reachable from the network. It uses a self-signed certificate, whose
fingerprint the connector logs at start (`docker logs remotehub-connector`), so the browser warns once. To use
a certificate of your own, see `REMOTEHUB_CONNECTOR_TLS_CERT_FILE` in
[configuration](configuration.md#site-connector).

Create a user for each person who opens and closes access. The password is shown once; `--totp` adds a
second factor for an authenticator app:

```bash
sudo docker exec remotehub-connector remotehub-connector user add anna --totp
```

On Windows, run the same commands in a PowerShell as administrator, e.g.
`& "C:\Program Files\remotehub-connector\remotehub-connector.exe" user add anna --totp`.

`user list`, `user reset NAME` and `user delete NAME` manage them. Five wrong attempts lock a name for five
minutes.

After signing in, the page shows whether access to the whole network is open, and offers to open it for 1, 4
or 8 hours, until a chosen time, or without end, and to close it. Below are the groups and devices, each with
its own switch, then the running connections and the latest log entries.

The same works on the command line, e.g. from a script or a scheduled task:

```bash
sudo docker exec remotehub-connector remotehub-connector open --hours 4
sudo docker exec remotehub-connector remotehub-connector open --until 2026-10-01T18:00:00+02:00
sudo docker exec remotehub-connector remotehub-connector open --permanent
sudo docker exec remotehub-connector remotehub-connector close
sudo docker exec remotehub-connector remotehub-connector status
```

### Open single devices

Suppose remotehub should reach only the servers of your ERP system, for a two-hour maintenance window. List
them on the connector, each with its address and the ports remotehub needs, and put them into a group:

```bash
sudo docker exec remotehub-connector remotehub-connector group add ERP
sudo docker exec remotehub-connector remotehub-connector device add haproxy --address 10.20.0.5 --ports 22 --group ERP
sudo docker exec remotehub-connector remotehub-connector device add sql --address sql01.corp.local --ports 3389,1433 --group ERP
sudo docker exec remotehub-connector remotehub-connector open --group ERP --hours 2
```

The web interface offers the same under *Groups* and *Devices*. An address is an IP, a range such as
`10.20.0.0/24`, or a host name, which the connector resolves in your network. A device can be in several
groups; `--group` may be repeated. New devices and groups start closed. `open --device NAME` opens a single
device, `close --group ERP` closes the group again. `device remove`, `group remove`, `device list` and
`group list` do what they say.

A connection goes through when its target is a listed port of an open device, alone or through an open group,
or anywhere while the whole network is open. `REMOTEHUB_CONNECTOR_ALLOW` still limits all of it. remotehub
shows such a connector as *open in part*, lists the open groups and devices with their end to its
administrators, marks each device as open or closed at the customer, and refuses devices you have not
opened.

### Ask the customer for access

A technician who needs a device behind a closed connector chooses *Ask the customer* on the device, or on
its folder for all the folder's devices behind the connector, with a duration and a reason. Within ten
seconds the request appears at the top of the connector's web interface, with each device's name in
remotehub, its address and port, and whether your own list knows that address. *Approve* opens exactly
these addresses and ports for the time asked; *Refuse* opens nothing. The page lists approved requests
until they end, each with *Close*. remotehub shows the answer and who gave it at the device.

Only a signed-in user of the web interface answers. A request waits a day, and the technician can withdraw
it before.

### The log

Every opening and closing, naming the device or group, every change to the list, every request from
remotehub with its answer, and every connection with its device, the remotehub user, its duration and the
bytes transferred, go to `access.log` in the data directory, one JSON object per line, and to the container's
log output. On Windows, the same lines also go to the event log (*Application*, source `remotehub-connector`):
ID 1 for openings, closings and requests, 2 for connections, 3 for sign-ins to the web interface; refused connections and
failed sign-ins are warnings. The remotehub user is the name remotehub reports; the connector cannot check it.
remotehub records openings and closings in its own audit log, too.

### Upgrade and tokens

remotehub refuses a connector of a release that speaks another protocol version; the connector's log then
names the version remotehub wants. After upgrading remotehub, start the connector again with the new release
and the same volume, or on Windows run `update` from the new release's file.

A token cannot be shown again. If it is lost or leaked, delete the connector and create a new one; its
devices must be moved to the new one first. Why the connector only carries connections and the engines stay
with remotehub: [ADR 0008](adr/0008-site-connectors.md).

## Back up and restore

Back up both databases regularly: `remotehub`, and `kratos` with the local accounts, their password hashes
and the keys of their authenticator apps. Back up `secrets/master_key` and `secrets/kratos.yml` once, apart
from them, and with Caddy of the package `caddy/data`, which holds its CA's key: a new CA means every client
must trust it anew.

```bash
sudo docker compose exec -T db pg_dump -U remotehub -Fc remotehub > remotehub-$(date +%F).dump
sudo docker compose exec -T db pg_dump -U remotehub -Fc kratos > kratos-$(date +%F).dump
```

To restore a backup into the running installation:

```bash
sudo docker compose stop remotehub kratos
sudo docker compose exec -T db pg_restore -U remotehub -d remotehub --clean --if-exists < remotehub-2026-01-31.dump
sudo docker compose exec -T db pg_restore -U remotehub -d kratos --clean --if-exists < kratos-2026-01-31.dump
sudo docker compose start kratos remotehub
```

The backup needs the `master_key` of the installation it came from. Once there is an organisation recovery key
(see [Recover personal vaults](#recover-personal-vaults)), the database holds a copy of `master_key` sealed for
it, and *Vault recovery* shows "Holds the master key". Without the file, restore it with the key's printed text:

```bash
sudo sh init.sh
sudo docker compose up -d db
sudo docker compose run --rm -T --no-deps remotehub recover-master-key | sudo tee secrets/master_key > /dev/null
```

`init.sh` creates the files a new host lacks, `master_key` among them, and the last command overwrites that one.
Type the printed text, then press Enter and Ctrl+D.

## Upgrade

1. Read the release notes on GitHub; they name every step a release needs beyond these.
2. Back up the database.
3. Set `REMOTEHUB_VERSION` in `.env` to the new release.
4. Pull and restart:

   ```bash
   sudo docker compose pull
   sudo docker compose up -d
   ```

remotehub updates the database schema when it starts. There is no way back to an older release except
restoring the backup made before the upgrade, together with the older `REMOTEHUB_VERSION`.
