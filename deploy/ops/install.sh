#!/bin/sh
# Installs remotehub on this host with one command (#142, docs/install.md):
#
#   curl -fsSL https://github.com/hilman2/remotehub/releases/latest/download/install.sh | sudo sh
#
# It asks for the domain only. On a host of its own, Caddy serves remotehub
# with a certificate from Let's Encrypt, or from a CA of its own where the
# internet cannot reach the host. With a reverse proxy on port 80 or 443
# already, remotehub is attached to that one. It ends with the link to the
# setup wizard. A second run keeps everything and shows the link again.
#
# Options, for installs without questions:
#   --domain NAME       the name people open, e.g. remotehub.example.com
#   --version X.Y.Z     a release other than the latest
#   --mode caddy|external
#                       caddy: Caddy of the package on ports 80 and 443;
#                       external: a reverse proxy of your own
#   --from URL          download the release from a mirror, which holds
#                       remotehub-ops.tar.gz and SHA256SUMS
#   --package FILE      an ops package on disk instead of the download
#   --dir DIR           where it goes (default /opt/remotehub)
set -eu

REPO=hilman2/remotehub
dir=/opt/remotehub
log=/var/log/remotehub-install.log
domain=""
version=latest
mode=""
package=""
from=""

say() {
  printf '%s\n' "$*"
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >>"$log" 2>/dev/null || true
}
warn() { say "warning: $*"; }
die() {
  say "error: $*"
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --domain) domain="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --mode) mode="$2"; shift 2 ;;
    --from) from="$2"; shift 2 ;;
    --package) package="$2"; shift 2 ;;
    --dir) dir="$2"; shift 2 ;;
    -h | --help) sed -n '2,22p' "$0"; exit 0 ;;
    *) die "unknown option $1" ;;
  esac
done
case "$mode" in "" | caddy | external) ;; *) die "--mode is caddy or external" ;; esac

# ── The host ────────────────────────────────────────────────────────────────

check_host() {
  [ "$(id -u)" = 0 ] || die "run it as root, e.g. with sudo"
  [ -r /etc/os-release ] || die "cannot tell the operating system (/etc/os-release)"
  # shellcheck disable=SC1091
  . /etc/os-release
  case "${ID:-}:${VERSION_ID:-}" in
    debian:12 | debian:13 | ubuntu:22.04 | ubuntu:24.04) ;;
    *) die "remotehub installs on Debian 12 or 13 and Ubuntu 22.04 or 24.04, not on ${PRETTY_NAME:-this system}" ;;
  esac
  [ "$(uname -m)" = x86_64 ] || die "remotehub's images exist for x86_64 only, not for $(uname -m)"
  command -v curl >/dev/null 2>&1 || die "curl is missing"
  memory=$(awk '/^MemTotal:/ { print int($2 / 1024) }' /proc/meminfo)
  [ "$memory" -ge 3800 ] || warn "only ${memory} MB of memory; remotehub wants 4 GB, the browser service alone up to 3 GB"
  space=$(df -Pm / | awk 'NR == 2 { print $4 }')
  [ "$space" -ge 20480 ] || warn "only ${space} MB free on /; remotehub wants 20 GB"
}

install_docker() {
  if ! command -v docker >/dev/null 2>&1; then
    say "Installing Docker from Docker's repository …"
    curl -fsSL https://get.docker.com | sh >>"$log" 2>&1 || die "installing Docker failed; see $log"
  fi
  docker compose version >/dev/null 2>&1 || die "the Docker Compose plugin is missing (docker-compose-plugin)"
  docker info >/dev/null 2>&1 || die "Docker does not run"
}

# Whether a terminal is there to ask on: the script itself comes on stdin.
has_terminal() { (exec </dev/tty) 2>/dev/null; }

ask_domain() {
  if [ -z "$domain" ]; then
    guess=$(hostname -f 2>/dev/null || hostname)
    case "$guess" in *.*) ;; *) guess="" ;; esac
    if has_terminal; then
      printf 'The name people open remotehub under [%s]: ' "$guess" >/dev/tty
      read -r domain </dev/tty || true
    fi
    domain=${domain:-$guess}
  fi
  domain=$(printf '%s' "$domain" | tr '[:upper:]' '[:lower:]')
  [ -n "$domain" ] || die "no domain: give one with --domain"
  case "$domain" in
    *[!a-z0-9.-]* | .* | *. | *..*) die "$domain is no host name" ;;
    *.*) ;;
    *) die "$domain has no dot; passkeys need a full name, e.g. remotehub.example.com" ;;
  esac
  case "$domain" in
    *[!0-9.]*) ;;
    *) die "$domain is an address; passkeys need a name" ;;
  esac
}

# ── The package ─────────────────────────────────────────────────────────────

fetch_package() {
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  if [ -n "$package" ]; then
    cp "$package" "$work/remotehub-ops.tar.gz"
  else
    case "$from:$version" in
      :latest) base="https://github.com/${REPO}/releases/latest/download" ;;
      :*) base="https://github.com/${REPO}/releases/download/v${version}" ;;
      *) base="${from%/}" ;;
    esac
    say "Downloading remotehub ($version) …"
    curl -fsSL "$base/remotehub-ops.tar.gz" -o "$work/remotehub-ops.tar.gz" ||
      die "cannot download $base/remotehub-ops.tar.gz"
    curl -fsSL "$base/SHA256SUMS" -o "$work/SHA256SUMS" || die "cannot download $base/SHA256SUMS"
    # The package's line, whether sha256sum marked it as text (" ") or as
    # binary ("*"); none at all fails too.
    (cd "$work" && grep '[ *]remotehub-ops[.]tar[.]gz$' SHA256SUMS | sha256sum -c - >/dev/null) ||
      die "the ops package does not match its checksum"
  fi
  tar -xzf "$work/remotehub-ops.tar.gz" -C "$work"
  mkdir -p "$dir"
  cp -R "$work/remotehub/." "$dir/"
}

# Sets KEY=VALUE in .env, in place of a line that sets it already.
set_env() {
  if grep -q "^#\{0,1\}$1=" "$dir/.env"; then
    sed -i "s|^#\{0,1\}$1=.*|$1=$2|" "$dir/.env"
  else
    printf '%s=%s\n' "$1" "$2" >>"$dir/.env"
  fi
}

# ── Next to a reverse proxy ─────────────────────────────────────────────────

# The program on port 80 or 443: caddy, nginx, traefik, or other; none if
# both are free.
proxy_on_web_ports() {
  listeners=$(ss -Hltnp 2>/dev/null | awk '$4 ~ /:(80|443)$/')
  [ -n "$listeners" ] || return 0
  case "$listeners" in
    *'"caddy"'*) echo caddy ;;
    *'"nginx"'*) echo nginx ;;
    *docker-proxy*)
      if docker ps --format '{{.Image}} {{.Ports}}' | grep -E ':(80|443)->' | grep -qi traefik; then
        echo traefik
      else
        echo other
      fi
      ;;
    *) echo other ;;
  esac
}

# A free port on 127.0.0.1 for remotehub, from 8080 on.
free_port() {
  port=8080
  while ss -Hltn "sport = :$port" | grep -q .; do port=$((port + 1)); done
  echo "$port"
}

reload() { # service
  if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet "$1"; then
    systemctl reload "$1"
  elif [ "$1" = nginx ]; then
    nginx -s reload
  else
    caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
  fi
}

attach_caddy() { # port
  site=/etc/caddy/remotehub.caddy
  [ -f /etc/caddy/Caddyfile ] || return 1
  cp /etc/caddy/Caddyfile /etc/caddy/Caddyfile.remotehub-backup
  printf '%s {\n\treverse_proxy 127.0.0.1:%s\n}\n' "$domain" "$1" >"$site"
  grep -qxF "import $site" /etc/caddy/Caddyfile || printf '\nimport %s\n' "$site" >>/etc/caddy/Caddyfile
  if caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile >>"$log" 2>&1 && reload caddy >>"$log" 2>&1; then
    say "Attached to Caddy on this host: $site"
    return 0
  fi
  mv /etc/caddy/Caddyfile.remotehub-backup /etc/caddy/Caddyfile
  rm -f "$site"
  return 1
}

attach_nginx() { # port
  conf=/etc/nginx/conf.d/remotehub.conf
  [ -d /etc/nginx/conf.d ] || return 1
  certificate="/etc/letsencrypt/live/$domain/fullchain.pem"
  key="/etc/letsencrypt/live/$domain/privkey.pem"
  if [ ! -f "$certificate" ]; then
    mkdir -p /etc/nginx/remotehub
    certificate=/etc/nginx/remotehub/remotehub.crt
    key=/etc/nginx/remotehub/remotehub.key
    [ -f "$certificate" ] ||
      openssl req -x509 -newkey rsa:3072 -nodes -days 825 -subj "/CN=$domain" \
        -addext "subjectAltName=DNS:$domain" -keyout "$key" -out "$certificate" >>"$log" 2>&1 ||
      return 1
  fi
  cat >"$conf" <<EOF
# remotehub (installed by install.sh): WebSockets carry terminals and
# desktops for hours.
map \$http_upgrade \$remotehub_connection_upgrade {
    default upgrade;
    ''      close;
}
server {
    listen 443 ssl;
    server_name $domain;
    ssl_certificate     $certificate;
    ssl_certificate_key $key;
    location / {
        proxy_pass http://127.0.0.1:$1;
        proxy_http_version 1.1;
        proxy_set_header Host \$host;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection \$remotehub_connection_upgrade;
        proxy_read_timeout 12h;
        proxy_send_timeout 12h;
    }
}
EOF
  if nginx -t >>"$log" 2>&1 && reload nginx >>"$log" 2>&1; then
    say "Attached to nginx on this host: $conf ($certificate)"
    return 0
  fi
  rm -f "$conf"
  return 1
}

attach_traefik() {
  container=$(docker ps --format '{{.ID}} {{.Image}} {{.Ports}}' | grep -E ':(80|443)->' | grep -i traefik | head -n 1 | cut -d' ' -f1)
  [ -n "$container" ] || return 1
  docker inspect -f '{{json .Args}} {{json .Config.Labels}}' "$container" | grep -q 'providers.docker' || return 1
  network=$(docker inspect -f '{{range $name, $_ := .NetworkSettings.Networks}}{{$name}} {{end}}' "$container" | cut -d' ' -f1)
  [ -n "$network" ] || return 1
  subnet=$(docker network inspect -f '{{range .IPAM.Config}}{{.Subnet}}{{end}}' "$network")
  cat >"$dir/compose.override.yml" <<EOF
# remotehub behind Traefik on this host (installed by install.sh).
services:
  remotehub:
    networks: [default, database, traefik]
    labels:
      traefik.enable: "true"
      traefik.docker.network: $network
      traefik.http.routers.remotehub.rule: Host(\`$domain\`)
      traefik.http.routers.remotehub.tls: "true"
      traefik.http.services.remotehub.loadbalancer.server.port: "8080"
networks:
  traefik:
    external: true
    name: $network
EOF
  set_env REMOTEHUB_EXTRA_PROXIES ",$subnet"
  say "Attached to Traefik ($network)"
}

print_proxy_settings() { # port
  say ""
  say "Point your reverse proxy at http://127.0.0.1:$1 for https://$domain:"
  say "  - pass WebSockets (Upgrade and Connection headers)"
  say "  - keep connections open for 12 hours"
  say "  - name the client in X-Forwarded-For"
  say "Examples: docs/install.md#reverse-proxy"
}

open_firewall() {
  if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q 'Status: active'; then
    ufw allow 80/tcp >>"$log" 2>&1 && ufw allow 443 >>"$log" 2>&1 && say "Opened ports 80 and 443 in ufw"
  fi
  if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
    firewall-cmd --permanent --add-service=http --add-service=https >>"$log" 2>&1 &&
      firewall-cmd --reload >>"$log" 2>&1 && say "Opened ports 80 and 443 in firewalld"
  fi
}

check_dns() {
  resolved=$(getent hosts "$domain" | awk '{ print $1 }' | head -n 1)
  own=$(hostname -I 2>/dev/null | awk '{ print $1 }')
  if [ -z "$resolved" ]; then
    warn "$domain does not resolve yet; enter it in DNS with the address ${own:-of this host}"
  elif [ -n "$own" ] && ! hostname -I | tr ' ' '\n' | grep -qxF "$resolved"; then
    warn "$domain resolves to $resolved, not to this host (${own})"
  fi
}

# ── Start and show the link ─────────────────────────────────────────────────

start_stack() {
  cd "$dir"
  say "Starting remotehub …"
  # Pulls what is missing; images given by REMOTEHUB_IMAGE and the like in
  # the environment, e.g. built for a test, are used as they are.
  docker compose up -d --wait --wait-timeout 300 >>"$log" 2>&1 ||
    die "remotehub did not start; see $log and: docker compose -f $dir/compose.yml logs"
}

# Waits until Caddy of the package serves remotehub over HTTPS, so that the
# link works once it is shown. Caddy tries Let's Encrypt first, which fails
# for a host the internet does not reach, and then takes its own CA.
wait_for_https() {
  say "Waiting for Caddy's certificate …"
  tries=0
  until curl -skf --resolve "$domain:443:127.0.0.1" "https://$domain/api/health" >/dev/null 2>&1; do
    tries=$((tries + 1))
    if [ "$tries" -ge 90 ]; then
      warn "Caddy does not serve https://$domain yet; see: docker compose -f $dir/compose.yml logs caddy"
      return
    fi
    sleep 2
  done
}

show_link() {
  cd "$dir"
  if link=$(docker compose exec -T remotehub remotehub setup-code 2>/dev/null | grep -o 'https\{0,1\}://[^ ]*/setup#code=[A-Za-z0-9_-]*'); then
    say ""
    say "remotehub is ready. Open this link to set it up:"
    say ""
    say "  $link"
    say ""
    say "It works until the setup has created the first administrator. A new one:"
    say "  sudo docker compose -f $dir/compose.yml exec remotehub remotehub setup-code"
  else
    say ""
    say "remotehub is set up and runs at https://$domain"
  fi
}

# ── Here it goes ────────────────────────────────────────────────────────────

mkdir -p "$(dirname "$log")"
check_host
install_docker

if [ -f "$dir/.env" ]; then
  # Installed before: nothing changes, not even the version.
  say "remotehub is installed in $dir already; keeping it as it is."
  domain=$(sed -n 's/^REMOTEHUB_HOST=//p' "$dir/.env" | tail -n 1)
  start_stack
  show_link
  exit 0
fi

ask_domain
say "Installing remotehub for https://$domain into $dir"
fetch_package
cd "$dir"
cp .env.example .env
set_env REMOTEHUB_PUBLIC_URL "https://$domain"
set_env REMOTEHUB_HOST "$domain"

if [ -z "$mode" ]; then
  found=$(proxy_on_web_ports)
  case "$found" in
    "") mode=caddy ;;
    *) mode=external ;;
  esac
fi
# On 127.0.0.1 only, for a proxy on this host; Caddy of the package reaches
# remotehub over the compose network instead.
set_env REMOTEHUB_PORT "$(free_port)"
if [ "$mode" = caddy ]; then
  set_env COMPOSE_PROFILES caddy
  mkdir -p caddy/data caddy/config
fi

sh init.sh >>"$log" 2>&1 || die "creating the secrets failed; see $log"
start_stack

if [ "$mode" = caddy ]; then
  open_firewall
  wait_for_https
  say "Caddy serves https://$domain: with Let's Encrypt if the internet reaches this host, otherwise with a CA of its own."
else
  port=$(sed -n 's/^REMOTEHUB_PORT=//p' .env | tail -n 1)
  case "${found:-other}" in
    caddy) attach_caddy "$port" || print_proxy_settings "$port" ;;
    nginx) attach_nginx "$port" || print_proxy_settings "$port" ;;
    traefik)
      if attach_traefik; then start_stack; else print_proxy_settings "$port"; fi
      ;;
    *) print_proxy_settings "$port" ;;
  esac
fi

check_dns
show_link
say "Files: $dir, log: $log"
