#!/usr/bin/env bash
# Tries the production images with the ops package the way an installation
# uses them: init.sh, docker compose up, then checks the running stack.
# Used by the local CI (job `image`) and by the release script.
#
#   image-check.sh DIR
#
# DIR is a fresh copy of deploy/ops at a path that the Docker daemon sees
# under the same name, since compose bind-mounts the secrets from there.
# The environment names the images: REMOTEHUB_IMAGE and GUACD_IMAGE
# (repositories), REMOTEHUB_VERSION (their tag), EXPECT_VERSION (what
# /api/health must report). Run it in a container: it joins the stack's
# network to reach the services.
set -euo pipefail

dir="$1"
: "${REMOTEHUB_IMAGE:?}" "${GUACD_IMAGE:?}" "${REMOTEHUB_VERSION:?}" "${EXPECT_VERSION:?}"
export REMOTEHUB_IMAGE GUACD_IMAGE REMOTEHUB_VERSION
export REMOTEHUB_PUBLIC_URL=http://localhost:8080
# The checks go through the stack's network; any free port on the host.
export REMOTEHUB_PORT=0
# No directory: remotehub starts without one; break-glass accounts work.
export REMOTEHUB_LDAP_URL='' REMOTEHUB_ADMIN_GROUPS=''
project="${COMPOSE_PROJECT_NAME:-remotehub-check}"
cd "$dir"

fail() {
  echo "FAILED: $*"
  docker compose -p "$project" ps -a || true
  docker compose -p "$project" logs --tail 40 || true
  exit 1
}

echo "── init.sh"
sh init.sh
for secret in db_password master_key ldap_bind_password; do
  [ -f "secrets/$secret" ] || fail "init.sh did not create secrets/$secret"
done
[ "$(stat -c %a secrets)" = 700 ] || fail "secrets/ is not 0700"
[ "$(stat -c %u:%g:%a secrets/master_key)" = 65532:65532:400 ] || fail "secrets/master_key is not 65532:65532, 0400"
[ "$(stat -c %u:%g:%a secrets/db_password)" = 65532:999:440 ] || fail "secrets/db_password is not 65532:999, 0440"
[ -s secrets/master_key ] || fail "secrets/master_key is empty"
sh init.sh | grep -q "kept    secrets/master_key" || fail "a second init.sh did not keep the secrets"

echo "── docker compose up"
docker compose -p "$project" up -d --quiet-pull --wait --wait-timeout 180 ||
  fail "the stack did not become healthy"

# This container joins the stack's network, where remotehub and guacd are.
network="${project}_default"
docker network connect "$network" "$(hostname)"

if docker compose -p "$project" logs remotehub | grep "master key file"; then
  fail "remotehub finds the master key file too open"
fi

echo "── remotehub answers"
health="$(curl -fsS http://remotehub:8080/api/health)" || fail "/api/health"
echo "$health"
grep -q "\"version\":\"${EXPECT_VERSION}\"" <<<"$health" || fail "version is not ${EXPECT_VERSION}"
grep -q '"database":"ok"' <<<"$health" || fail "the database is not reachable"
curl -fsS http://remotehub:8080/ | grep -q '<html' || fail "the web UI is not served"
curl -fsS http://remotehub:8080/devices/any | grep -q '<html' || fail "routes of the SPA do not fall back to index.html"

echo "── Hardening"
container="$(docker compose -p "$project" ps -q remotehub)"
[ "$(docker inspect -f '{{.Config.User}}' "$container")" = 65532:65532 ] || fail "remotehub does not run as 65532"
[ "$(docker inspect -f '{{.HostConfig.ReadonlyRootfs}}' "$container")" = true ] || fail "remotehub's file system is writable"
users="$(docker top "$container" -o pid,uid | awk 'NR > 1 { print $2 }' | sort -u)"
[ "$users" = 65532 ] || fail "processes in remotehub run as: ${users}"
if timeout 3 bash -c '</dev/tcp/db/5432' 2>/dev/null; then
  fail "the database is reachable from outside its network"
fi
timeout 3 bash -c '</dev/tcp/guacd/4822' || fail "guacd is not reachable"

echo "── CLI with the secrets"
docker compose -p "$project" exec -T remotehub remotehub break-glass create check >/dev/null ||
  fail "break-glass create"
docker compose -p "$project" exec -T remotehub remotehub verify-audit | tee /dev/stderr |
  grep -q "audit log intact" || fail "verify-audit"

echo "── Restart keeps the data"
docker compose -p "$project" restart remotehub
docker compose -p "$project" up -d --wait --wait-timeout 120 || fail "remotehub did not come back"
docker compose -p "$project" exec -T remotehub remotehub break-glass list | grep -qx check ||
  fail "the break-glass account is gone after a restart"

echo "── Backup and restore (docs/install.md)"
docker compose -p "$project" exec -T db pg_dump -U remotehub -Fc remotehub >/tmp/remotehub.dump
docker compose -p "$project" exec -T remotehub remotehub break-glass delete check >/dev/null
docker compose -p "$project" stop remotehub
docker compose -p "$project" exec -T db pg_restore -U remotehub -d remotehub --clean --if-exists </tmp/remotehub.dump ||
  fail "pg_restore"
docker compose -p "$project" start remotehub
docker compose -p "$project" up -d --wait --wait-timeout 120 || fail "remotehub did not come back after the restore"
docker compose -p "$project" exec -T remotehub remotehub break-glass list | grep -qx check ||
  fail "the restore did not bring the break-glass account back"
docker compose -p "$project" exec -T remotehub remotehub verify-audit >/dev/null || fail "verify-audit after the restore"

docker network disconnect "$network" "$(hostname)"
docker compose -p "$project" down -v
echo "images ok: ${REMOTEHUB_IMAGE}:${REMOTEHUB_VERSION}, ${GUACD_IMAGE}:${REMOTEHUB_VERSION}"
