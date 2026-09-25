#!/bin/sh
# Prepares a new installation next to compose.yml: .env from .env.example,
# and the secrets in secrets/ (docs/install.md).
# Run it once as root, where docker compose will run. It never overwrites a
# file.
set -eu
cd "$(dirname "$0")"

[ -f .env ] || cp .env.example .env

# The image to ask for a master key: from the environment, else from .env.
setting() { # name
  eval "value=\${$1:-}"
  # shellcheck disable=SC2154 # set by eval
  [ -n "$value" ] || value="$(sed -n "s/^$1=//p" .env | tail -n 1)"
  printf '%s' "$value"
}
image="$(setting REMOTEHUB_IMAGE)"
image="${image:-ghcr.io/hilman2/remotehub}:$(setting REMOTEHUB_VERSION)"

# Compose mounts the files as they are, so they belong to the users in the
# containers: remotehub runs as 65532, PostgreSQL as 999 (group 999), Kratos
# as 10000.
if [ "$(id -u)" != 0 ]; then
  echo "Run init.sh as root: the secrets must belong to the containers' users." >&2
  exit 1
fi
mkdir -p secrets
chmod 700 secrets
umask 077

new_secret() { # file owner mode command...
  file="secrets/$1" owner="$2" mode="$3"
  shift 3
  if [ -e "$file" ]; then
    echo "kept    $file"
    return
  fi
  "$@" >"$file.new"
  chown "$owner" "$file.new"
  chmod "$mode" "$file.new"
  mv "$file.new" "$file"
  echo "created $file"
}
random_password() { # length
  LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c "${1:-40}"
}
# Kratos' database and secrets, as a second configuration file next to
# kratos/kratos.yml; its cipher secret has exactly 32 characters. It hands
# its mails to remotehub with the courier token (#145).
kratos_config() {
  cat <<EOF
# Ory Kratos: its database and secrets (docs/install.md#local-accounts).
# Sign-in providers such as Entra ID go here too (docs/install.md#sign-in-with-entra-id-google-or-github).
dsn: postgres://remotehub:$(cat secrets/db_password)@db:5432/kratos?sslmode=disable
secrets:
  cookie: ["$(random_password)"]
  cipher: ["$(random_password 32)"]
  default: ["$(random_password)"]
courier:
  http:
    request_config:
      auth:
        type: api_key
        config:
          name: Authorization
          value: Bearer $(cat secrets/courier_token)
          in: header
EOF
}

new_secret db_password 65532:999 440 random_password
new_secret master_key 65532:65532 400 docker run --rm "$image" generate-key
new_secret ssh_ca_key 65532:65532 400 docker run --rm "$image" generate-ssh-ca
new_secret courier_token 65532:65532 400 random_password
new_secret kratos.yml 10000:10000 400 kratos_config

echo
echo "Next: set REMOTEHUB_PUBLIC_URL in .env, then: docker compose up -d"
echo "and open the link of: docker compose exec remotehub remotehub setup-code"
echo "Back up secrets/master_key apart from the database: without it the"
echo "vault cannot be read."
