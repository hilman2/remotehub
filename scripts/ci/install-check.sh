#!/usr/bin/env bash
# Tries install.sh (#142) as a host runs it, with the images image-check.sh
# built. The host is a Debian container that shares the network of a
# Docker-in-Docker daemon and its /opt/remotehub, so that ports and bind
# mounts look alike on both sides. Three hosts, each with a fresh daemon:
#
#   own     nothing on ports 80 and 443: Caddy of the package serves remotehub
#           with a certificate of its own CA; a second run changes nothing
#   caddy   Caddy on the host already: remotehub is attached to it
#   nginx   nginx on the host already: remotehub is attached to it
#
#   install-check.sh REMOTEHUB_IMAGE GUACD_IMAGE BROWSER_IMAGE TAG
#
# Run it like image-check.sh: in a directory the Docker daemon sees under the
# same path.
set -euo pipefail

image="$1" guacd="$2" browser="$3" tag="$4"
project="${COMPOSE_PROJECT_NAME:-remotehub-install-check}"
host_image="remotehub-ci-install-host"
dind_image="docker:29.8.1-dind"
work="${PWD}/.install-check"

echo "── Host image and ops package"
docker build --quiet --tag "$host_image" --file scripts/ci/install-host.Dockerfile scripts/ci >/dev/null
rm -rf "$work"
mkdir -p "$work"
# As release.sh packs it: the directory `remotehub` with deploy/ops in it.
tar -C deploy -czf "${work}/remotehub-ops.tar.gz" --transform 's,^ops,remotehub,' ops

fail() {
  echo "FAILED: $*"
  exit 1
}

# Runs `script` on a fresh host with Docker-in-Docker; `prepare` runs before
# the installer, `check` after it.
host() { # name prepare check
  local name="$1" prepare="$2" check="$3"
  local dind="${project}-${name}" opt="${project}-${name}-opt"
  echo "── Host: ${name}"
  docker volume create "$opt" >/dev/null
  docker run -d --privileged --name "$dind" -e DOCKER_TLS_CERTDIR= \
    -v "${opt}:/opt/remotehub" "$dind_image" >/dev/null
  cleanup() {
    docker rm -f "$dind" >/dev/null 2>&1 || true
    docker volume rm "$opt" >/dev/null 2>&1 || true
  }
  for _ in $(seq 1 60); do
    docker exec "$dind" docker info >/dev/null 2>&1 && break
    sleep 1
  done
  docker save "${image}:${tag}" "${guacd}:${tag}" "${browser}:${tag}" | docker exec -i "$dind" docker load >/dev/null
  local rc=0
  docker run --rm --network "container:${dind}" -v "${opt}:/opt/remotehub" \
    -v "${work}:/install:ro" -v "${PWD}/deploy/ops/install.sh:/install.sh:ro" \
    -e DOCKER_HOST=tcp://127.0.0.1:2375 \
    -e REMOTEHUB_IMAGE="$image" -e GUACD_IMAGE="$guacd" -e BROWSER_IMAGE="$browser" \
    -e REMOTEHUB_VERSION="$tag" \
    "$host_image" bash -euo pipefail -c "
      ${prepare}
      sh /install.sh --domain remotehub.test --package /install/remotehub-ops.tar.gz | tee /tmp/first.log ||
        { tail -n 40 /var/log/remotehub-install.log; exit 1; }
      grep -q 'https://remotehub.test/setup#code=' /tmp/first.log || { echo 'FAILED: no setup link'; exit 1; }
      ${check}
    " || rc=$?
  if [ "$rc" != 0 ]; then
    docker exec "$dind" sh -c 'docker ps -a; for c in $(docker ps -aq); do docker logs --tail 30 $c 2>&1; done' || true
    cleanup
    fail "host ${name}"
  fi
  cleanup
}

# What people get: remotehub's health, through the proxy, over HTTPS.
health='curl -skf --resolve remotehub.test:443:127.0.0.1 https://remotehub.test/api/health | grep -q "\"database\":\"ok\"" || { echo "FAILED: no remotehub behind the proxy"; exit 1; }'

host own "" "
  ${health}
  grep -qx 'COMPOSE_PROFILES=caddy' /opt/remotehub/.env || { echo 'FAILED: Caddy of the package is not on'; exit 1; }
  echo | openssl s_client -connect 127.0.0.1:443 -servername remotehub.test 2>/dev/null |
    openssl x509 -noout -issuer | grep -q 'remotehub remotehub.test Intermediate' ||
    { echo 'FAILED: the certificate is not from the CA named after the host'; exit 1; }
  # Caddy runs without capabilities and reads what remotehub writes through
  # the group root (#146).
  [ \"\$(stat -c '%a %u:%g' /opt/remotehub/caddy/remotehub/tls.caddy)\" = '640 65532:0' ] ||
    { echo 'FAILED: the snippet for Caddy is not 640 65532:0'; ls -ln /opt/remotehub/caddy/remotehub; exit 1; }
  before=\$(sha256sum /opt/remotehub/.env /opt/remotehub/secrets/* | sha256sum)
  sh /install.sh | tee /tmp/second.log
  grep -q 'installed in /opt/remotehub already' /tmp/second.log || { echo 'FAILED: a second run did not keep the installation'; exit 1; }
  [ \"\$before\" = \"\$(sha256sum /opt/remotehub/.env /opt/remotehub/secrets/* | sha256sum)\" ] ||
    { echo 'FAILED: a second run changed .env or the secrets'; exit 1; }
"

host caddy "
  mkdir -p /etc/caddy
  printf '{\n\tlocal_certs\n}\n:80 {\n\trespond \"another site\"\n}\n' >/etc/caddy/Caddyfile
  caddy start --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null 2>&1
" "
  ${health}
  grep -q 'reverse_proxy 127.0.0.1:' /etc/caddy/remotehub.caddy || { echo 'FAILED: no site block'; exit 1; }
  grep -q '^COMPOSE_PROFILES=caddy' /opt/remotehub/.env && { echo 'FAILED: Caddy of the package is on next to Caddy'; exit 1; }
  true
"

host nginx "
  nginx
" "
  ${health}
  nginx -T 2>/dev/null | grep -q 'server_name remotehub.test' || { echo 'FAILED: no server block'; exit 1; }
"

rm -rf "$work"
echo "install ok: own host, next to Caddy, next to nginx"
