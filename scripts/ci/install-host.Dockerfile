# A host for install-check.sh (#142): Debian as install.sh supports it, with
# Docker's client and Compose plugin (the daemon is Docker-in-Docker next to
# it), and Caddy and nginx to install next to. None of them runs yet.
FROM debian:13
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates curl iproute2 openssl procps \
      docker-cli docker-compose caddy nginx \
 && rm -rf /var/lib/apt/lists/*
