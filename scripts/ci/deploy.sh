#!/usr/bin/env bash
# Upgrades an installation of remotehub over SSH to a release (#225), the way
# docs/install.md#upgrade says:
#
#   bash scripts/ci/deploy.sh HOST DIR X.Y.Z
#   bash scripts/ci/deploy.sh remotehub-host /opt/remotehub 0.4.0
#
# On HOST (`ssh HOST`, with sudo without a password), in DIR, the ops
# package's directory: the release's ops package, checked against its
# SHA256SUMS, a backup of both databases into DIR/backups, the package's own
# files of the new release (#235), the new REMOTEHUB_VERSION in .env, pull,
# start, and a check that /api/health reports the version. The release must
# be published. .env, secrets/, compose.override.yml and the data stay.
set -euo pipefail

host="${1:-}" dir="${2:-}" version="${3:-}"
if [ -z "$host" ] || [ -z "$dir" ] || ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Usage: bash scripts/ci/deploy.sh HOST DIR X.Y.Z" >&2
  exit 2
fi

ssh -o BatchMode=yes "$host" sudo -n bash -s -- "$dir" "$version" <<'REMOTE'
set -euo pipefail
dir="$1" version="$2"
cd "$dir"
if [ ! -f .env ] || [ ! -f compose.yml ]; then
  echo "no installation of remotehub in ${dir}" >&2
  exit 1
fi

# The release's ops package, checked, before anything changes here (#235).
owned=(compose.yml kratos caddy/Caddyfile init.sh install.sh .env.example)
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
release="https://github.com/hilman2/remotehub/releases/download/v${version}"
curl -fsSL "${release}/remotehub-ops.tar.gz" -o "${work}/remotehub-ops.tar.gz"
curl -fsSL "${release}/SHA256SUMS" -o "${work}/SHA256SUMS"
(cd "$work" && grep '[ *]remotehub-ops[.]tar[.]gz$' SHA256SUMS | sha256sum -c - >/dev/null)
tar -xzf "${work}/remotehub-ops.tar.gz" -C "$work"
for file in "${owned[@]}"; do
  [ -e "${work}/remotehub/${file}" ] || { echo "the ops package of ${version} has no ${file}" >&2; exit 1; }
done

# This script reaches bash on stdin: docker compose reads stdin too, and
# would take the rest of the script with it (#229).
# There is no way back to an older release but a backup (docs/install.md#upgrade).
stamp="$(date +%Y%m%d-%H%M%S)"
mkdir -p backups
chmod 700 backups
for database in remotehub kratos; do
  docker compose exec -T db pg_dump -U remotehub -Fc "$database" </dev/null \
    >"backups/${database}-${stamp}.dump"
done
echo "backup: ${dir}/backups/{remotehub,kratos}-${stamp}.dump"

for file in "${owned[@]}"; do
  mkdir -p "$(dirname "$file")"
  rm -rf "$file"
  cp -R "${work}/remotehub/${file}" "$file"
done
echo "ops package ${version}: ${owned[*]}"

before="$(sed -n 's/^REMOTEHUB_VERSION=//p' .env | tail -n 1)"
sed -i "s/^REMOTEHUB_VERSION=.*/REMOTEHUB_VERSION=${version}/" .env
docker compose pull -q </dev/null
docker compose up -d --wait --wait-timeout 300 </dev/null

port="$(sed -n 's/^REMOTEHUB_PORT=//p' .env | tail -n 1)"
health="$(curl -fsS "http://127.0.0.1:${port:-8080}/api/health")"
case "$health" in
  *"\"version\":\"${version}\""*) echo "remotehub ${before} → ${version}: ${health}" ;;
  *)
    echo "/api/health does not report ${version}: ${health}" >&2
    exit 1
    ;;
esac
REMOTE
