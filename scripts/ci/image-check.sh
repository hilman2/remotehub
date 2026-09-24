#!/usr/bin/env bash
# Builds the production images from the source in the current directory and
# tries them with the ops package the way an installation uses them:
# init.sh, docker compose up, then checks on the running stack. The local CI
# (job `image`) and the release script run it alike.
#
#   image-check.sh VERSION REMOTEHUB_IMAGE GUACD_IMAGE BROWSER_IMAGE TAG [docker build options]
#
# VERSION is the release (Cargo.toml), which /api/health must report; all
# images are tagged TAG. Run it in a container, in a directory that the
# Docker daemon sees under the same path (the run's volume, see
# gemeinsam.sh): compose bind-mounts the secrets from there, and the
# container joins the stack's network to reach the services.
set -euo pipefail

version="$1" image="$2" guacd="$3" browser_image="$4" tag="$5"
shift 5

echo "── docker build (version ${version})"
docker build --quiet "$@" --file deploy/Dockerfile --build-arg VERSION="$version" --tag "${image}:${tag}" .
# The source label links the package on GHCR to the repository.
docker build --quiet "$@" --tag "${guacd}:${tag}" \
  --label org.opencontainers.image.source=https://github.com/hilman2/remotehub \
  --label "org.opencontainers.image.version=${version}" deploy/guacd
docker build --quiet "$@" --file deploy/browser/Dockerfile --build-arg VERSION="$version" \
  --tag "${browser_image}:${tag}" .

dir="${PWD}/.image-check"
rm -rf "$dir"
cp -r deploy/ops "$dir"
export REMOTEHUB_IMAGE="$image" GUACD_IMAGE="$guacd" BROWSER_IMAGE="$browser_image" REMOTEHUB_VERSION="$tag"
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
for secret in db_password master_key ssh_ca_key ldap_bind_password; do
  [ -f "secrets/$secret" ] || fail "init.sh did not create secrets/$secret"
done
[ "$(stat -c %a secrets)" = 700 ] || fail "secrets/ is not 0700"
[ "$(stat -c %u:%g:%a secrets/master_key)" = 65532:65532:400 ] || fail "secrets/master_key is not 65532:65532, 0400"
[ "$(stat -c %u:%g:%a secrets/db_password)" = 65532:999:440 ] || fail "secrets/db_password is not 65532:999, 0440"
[ "$(stat -c %u:%g:%a secrets/ssh_ca_key)" = 65532:65532:400 ] || fail "secrets/ssh_ca_key is not 65532:65532, 0400"
[ -s secrets/master_key ] || fail "secrets/master_key is empty"
sh init.sh | grep -q "kept    secrets/master_key" || fail "a second init.sh did not keep the secrets"

echo "── docker compose up"
docker compose -p "$project" up -d --quiet-pull --wait --wait-timeout 180 ||
  fail "the stack did not become healthy"

# This container joins the stack's network, where remotehub and guacd are.
network="${project}_default"
docker network connect "$network" "$(hostname)"
# A service's address on one of the stack's networks. Its name would be
# ambiguous here: the local CI's own network has a db and a guacd, too.
address() { # service network
  docker inspect -f "{{(index .NetworkSettings.Networks \"${project}_$2\").IPAddress}}" \
    "$(docker compose -p "$project" ps -q "$1")"
}
remotehub="$(address remotehub default)"

if docker compose -p "$project" logs remotehub | grep "accessible to other users"; then
  fail "remotehub finds a key file too open"
fi

echo "── remotehub answers"
health="$(curl -fsS "http://${remotehub}:8080/api/health")" || fail "/api/health"
echo "$health"
grep -q "\"version\":\"${version}\"" <<<"$health" || fail "version is not ${version}"
grep -q '"database":"ok"' <<<"$health" || fail "the database is not reachable"
curl -fsS "http://${remotehub}:8080/" | grep -q '<html' || fail "the web UI is not served"
curl -fsS "http://${remotehub}:8080/api/ssh-ca.pub" | grep -q '^ssh-ed25519 ' ||
  fail "the SSH CA's public key is not served"
curl -fsS "http://${remotehub}:8080/devices/any" | grep -q '<html' ||
  fail "routes of the SPA do not fall back to index.html"

echo "── Hardening"
container="$(docker compose -p "$project" ps -q remotehub)"
[ "$(docker inspect -f '{{.Config.User}}' "$container")" = 65532:65532 ] || fail "remotehub does not run as 65532"
[ "$(docker inspect -f '{{.HostConfig.ReadonlyRootfs}}' "$container")" = true ] || fail "remotehub's file system is writable"
users="$(docker top "$container" -o pid,uid | awk 'NR > 1 { print $2 }' | sort -u)"
[ "$users" = 65532 ] || fail "processes in remotehub run as: ${users}"
db_networks="$(docker inspect -f '{{range $name, $_ := .NetworkSettings.Networks}}{{$name}} {{end}}' \
  "$(docker compose -p "$project" ps -q db)")"
[ "$db_networks" = "${project}_database " ] || fail "the database is on the networks: ${db_networks}"
[ "$(docker network inspect -f '{{.Internal}}' "${project}_database")" = true ] ||
  fail "the database's network has a way out"
guacd="$(address guacd default)"
timeout 3 bash -c "</dev/tcp/${guacd}/4822" || fail "guacd is not reachable"
container="$(docker compose -p "$project" ps -q browser)"
[ "$(docker inspect -f '{{.HostConfig.ReadonlyRootfs}}' "$container")" = true ] || fail "the browser's file system is writable"
users="$(docker top "$container" -o pid,uid | awk 'NR > 1 { print $2 }' | sort -u)"
[ "$users" = 10001 ] || fail "processes in the browser service run as: ${users}"

echo "── The browser service opens a page"
# Chromium starts with its sandbox only as compose.yml runs it. Asked for a
# page on remotehub's plain HTTP port, it shows its error page: the agent
# answers ready, then page_error. A Chromium that did not start answers
# something else.
open_page() { # browser
  exec 3<>"/dev/tcp/$1/4823"
  printf '%s\n' '{"host":"remotehub","port":8080,"spki":"AAAA","width":800,"height":600,"login":{"username":"x","password":"x"}}' >&3
  head -n 2 <&3
}
answer="$(timeout 60 bash -c "$(declare -f open_page); open_page $(address browser default)")" || true
echo "$answer"
grep -q '"type":"ready"' <<<"$answer" && grep -q '"reason":"page_error"' <<<"$answer" ||
  fail "the browser service did not show a page"

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

echo "── The client's address behind a proxy"
# A failed break-glass sign-in logs the address remotehub took for the
# client. Through the published port, as a proxy on the host connects, the
# forwarded address counts; from anywhere else, it does not.
break_glass_attempt() { # forwarded-for url
  curl -s -o /dev/null -X POST -H 'Content-Type: application/json' -H "X-Forwarded-For: $1" \
    -d '{"username":"nobody","password":"x","code":"000000"}' "$2/api/session/break-glass"
}
published="$(docker compose -p "$project" port remotehub 8080)"
docker run --rm --network host "$(docker inspect -f '{{.Config.Image}}' "$(hostname)")" \
  bash -c "$(declare -f break_glass_attempt); break_glass_attempt 203.0.113.9 http://${published}"
break_glass_attempt 203.0.113.10 "http://${remotehub}:8080"
failures="$(docker compose -p "$project" logs remotehub | grep "break-glass sign-in failed")"
grep -q "203.0.113.9" <<<"$failures" || fail "the proxy's forwarded address was not taken: ${failures}"
if grep -q "203.0.113.10" <<<"$failures"; then
  fail "a forwarded address from an untrusted peer was taken"
fi

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
echo "images ok: ${REMOTEHUB_IMAGE}:${REMOTEHUB_VERSION}, ${GUACD_IMAGE}:${REMOTEHUB_VERSION}, ${BROWSER_IMAGE}:${REMOTEHUB_VERSION}"
