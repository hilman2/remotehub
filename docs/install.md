# Installing remotehub

remotehub runs as three containers: the server, PostgreSQL and guacd, the engine for RDP and VNC. The ops
package in [`deploy/ops`](../deploy/ops) starts them with Docker Compose. Every setting is described in the
[configuration reference](configuration.md).

You need a Linux host with Docker Engine and the Compose plugin, a DNS name for remotehub, a TLS certificate
for it, and a service account in Active Directory that may read users and groups.

## Install

Copy `deploy/ops` of the release you want to the host, e.g. to `/opt/remotehub`, and prepare it as root:

```bash
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
| `ldap_bind_password` | The directory service account's password. Empty until you fill it in. | 65532, 0400 |

remotehub runs as user 65532 and PostgreSQL as 999, so the files belong to them; `secrets/` itself is open
to root only. Keep these owners and modes when you edit a file, e.g. with `sudo tee secrets/ldap_bind_password`.

Without `master_key`, the credentials in the database cannot be read by anyone. Keep a copy of it apart from
the database backups.

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
reaches it. The network uses `10.213.213.0/24`; guacd cannot reach devices in that range. If your network uses
it, set another range in `.env` before the first start:

```
REMOTEHUB_SUBNET=10.99.99.0/24
REMOTEHUB_GATEWAY=10.99.99.1
```

A proxy elsewhere than on this host goes into `REMOTEHUB_TRUSTED_PROXIES` in `compose.yml` instead.

## First sign-in

Members of `REMOTEHUB_ADMIN_GROUPS` sign in with their directory account and administer everything. For the
day the directory is unreachable, create a break-glass account:

```bash
sudo docker compose exec remotehub remotehub break-glass create emergency
```

Its password and TOTP secret are shown only this once. Keep them offline, e.g. in a safe. It signs in at
`/sign-in/break-glass`.

## Back up and restore

Back up the database regularly, and `secrets/master_key` once, apart from it:

```bash
sudo docker compose exec -T db pg_dump -U remotehub -Fc remotehub > remotehub-$(date +%F).dump
```

To restore a backup into the running installation:

```bash
sudo docker compose stop remotehub
sudo docker compose exec -T db pg_restore -U remotehub -d remotehub --clean --if-exists < remotehub-2026-01-31.dump
sudo docker compose start remotehub
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
