# Installing remotehub

remotehub runs as four containers: the server, PostgreSQL, guacd, the engine for RDP and VNC, and the browser
service, which opens web interfaces of devices. The ops package in [`deploy/ops`](../deploy/ops) starts them
with Docker Compose. Every setting is described in the
[configuration reference](configuration.md).

You need a Linux host with Docker Engine and the Compose plugin, a DNS name for remotehub, a TLS certificate
for it, and a service account in Active Directory that may read users and groups.

## Install

Download the ops package `remotehub-ops-X.Y.Z.tar.gz` of the
[release](https://github.com/hilman2/remotehub/releases) you want, unpack it on the host (it holds the
directory `remotehub`) and prepare it as root:

```bash
sudo tar -xzf remotehub-ops-0.1.0.tar.gz -C /opt
cd /opt/remotehub
sudo sh init.sh
```

`init.sh` copies `.env.example` to `.env` and creates the secrets (see below). Then edit `.env`:

- `REMOTEHUB_PUBLIC_URL`: the address people will open, with `https://`.
- `REMOTEHUB_LDAP_URL`, `REMOTEHUB_LDAP_BIND_DN`, `REMOTEHUB_LDAP_BASE_DN`: your directory.
- `REMOTEHUB_ADMIN_GROUPS`: the group whose members administer remotehub.

Put the service account's password into `secrets/ldap_bind_password`. If the directory's certificate comes
from your own CA, save that CA as `certs/ldap-ca.crt` and add `REMOTEHUB_LDAP_CA_FILE=/run/certs/ldap-ca.crt`
to `.env`.

Start it:

```bash
sudo docker compose up -d
sudo docker compose ps
```

After a few seconds all three services are up, and remotehub reports `healthy`. It listens on
`127.0.0.1:8080`; set up the reverse proxy next.

## Secrets

`init.sh` creates these files in `secrets/` once and never overwrites them:

| File | Holds | Owner, mode |
|---|---|---|
| `master_key` | The key that encrypts every stored credential. | 65532, 0400 |
| `db_password` | The database password, read by PostgreSQL and remotehub. | 65532:999, 0440 |
| `ssh_ca_key` | The key of remotehub's SSH certificate authority. | 65532, 0400 |
| `ldap_bind_password` | The directory service account's password. Empty until you fill it in. | 65532, 0400 |
| `kratos.yml` | Database and secrets of Kratos, which keeps the local accounts; the SMTP server goes here too. | 10000, 0400 |

remotehub runs as user 65532, PostgreSQL as 999 and Kratos as 10000, so the files belong to them; `secrets/`
itself is open to root only. Keep these owners and modes when you edit a file, e.g. with `sudo tee secrets/ldap_bind_password`.

Without `master_key`, the credentials in the database cannot be read by anyone. Keep a copy of it apart from
the database backups. The same goes for `ssh_ca_key`: a new one means every device must trust the new
public key.

An installation from 0.1.0 has no `ssh_ca_key` and no `kratos.yml` yet; running `sudo sh init.sh` again
creates them and keeps the other files.

## Reverse proxy

remotehub speaks plain HTTP on `127.0.0.1:8080`. A reverse proxy on the host serves it under
`REMOTEHUB_PUBLIC_URL` with TLS. It must pass WebSockets, which carry the terminal and the remote desktops,
and keep them open for hours.

With Caddy, which gets the certificate itself:

```
remotehub.example.com {
	reverse_proxy 127.0.0.1:8080
}
```

With nginx:

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

The proxy names the client in `X-Forwarded-For`; Caddy does that by itself, nginx with the line above.
remotehub believes that header only from the gateway of its Docker network, which is how a proxy on the host
reaches it. The network uses `10.213.213.0/24`; guacd and the browser service cannot reach devices in that
range. If your network uses
it, set another range in `.env` before the first start:

```
REMOTEHUB_SUBNET=10.99.99.0/24
REMOTEHUB_GATEWAY=10.99.99.1
```

A proxy elsewhere than on this host goes into `REMOTEHUB_TRUSTED_PROXIES` in `compose.yml` instead.

## First sign-in

Members of `REMOTEHUB_ADMIN_GROUPS` sign in with their directory account and administer everything.

Without a directory, invite the first administrator as a local account. Put the address into
`REMOTEHUB_ADMIN_ACCOUNTS` in `.env`, run `docker compose up -d`, then:

```bash
sudo docker compose exec remotehub remotehub account invite you@example.com --name "Your Name"
```

It prints a link and a one-time code, valid for 48 hours. Open the link, enter the code, choose a password
and set up an authenticator app: remotehub asks for its code at every sign-in.

For the day the directory or Kratos is unreachable, create a break-glass account:

```bash
sudo docker compose exec remotehub remotehub break-glass create emergency
```

Its password and TOTP secret are shown only this once. Keep them offline, e.g. in a safe. It signs in at
`/sign-in/break-glass`.

## Local accounts

Local accounts live in Ory Kratos, which the ops package runs next to remotehub. People reach it only
through remotehub. Its settings are in `kratos/kratos.yml`; database and secrets in `secrets/kratos.yml`.

- **Inviting and managing:** administrators invite local accounts under *Users* and hand over the link
  and code shown there. The same page blocks anyone, local or from the directory, ends their sessions,
  and deletes local accounts. Everything done there is in the audit log.
- **Groups:** without a directory, put local accounts into groups under *Users* and grant access to the
  groups. Directory users and groups can be members too.
- **Forgotten passwords:** "Forgot the password?" on the sign-in page mails a code. For that, add your SMTP
  server to `secrets/kratos.yml` as its comment shows, then `docker compose up -d kratos`. Without it, an
  administrator gives the person a *New sign-in code* under *Users*.
- **Second factor:** every local account sets up an authenticator app before its first session, and can
  create recovery codes under *My account*. A lost authenticator app takes a *New sign-in code* too: it
  removes the second factor, and the person sets up a new one with the code.
- **Backups:** Kratos' database sits next to remotehub's; see [Back up and restore](#back-up-and-restore).

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
HTTPS to `REMOTEHUB_PUBLIC_URL` and nothing inbound. It is the remotehub image, started as `connector`.

Create the connector under *Connectors* in remotehub. Its token is shown once. On a Linux host in that network,
put the token into a file and start the connector:

```bash
sudo install -d -m 700 /opt/remotehub-connector
sudo sh -c 'cat > /opt/remotehub-connector/token' # paste the token, then Ctrl+D
sudo chown 65532 /opt/remotehub-connector/token && sudo chmod 400 /opt/remotehub-connector/token
sudo docker run -d --name remotehub-connector --restart unless-stopped --read-only --no-healthcheck \
  --cap-drop ALL --security-opt no-new-privileges \
  -v /opt/remotehub-connector/token:/run/secrets/token:ro \
  -e REMOTEHUB_URL=https://remotehub.example.com \
  -e REMOTEHUB_CONNECTOR_TOKEN_FILE=/run/secrets/token \
  ghcr.io/hilman2/remotehub:0.1.0 connector
```

The connectors page shows it as connected. Devices in that network then name it under *Reached through*. To
keep the connector away from parts of its network, list the ranges it may reach in `REMOTEHUB_CONNECTOR_ALLOW`.

A token cannot be shown again. If it is lost or leaked, delete the connector and create a new one; its
devices must be moved to the new one first. Why the connector only carries connections and the engines stay
with remotehub: [ADR 0008](adr/0008-site-connectors.md).

## Back up and restore

Back up both databases regularly: `remotehub`, and `kratos` with the local accounts, their password hashes
and the keys of their authenticator apps. Back up `secrets/master_key` and `secrets/kratos.yml` once, apart
from them:

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

The backup needs the `master_key` of the installation it came from.

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
