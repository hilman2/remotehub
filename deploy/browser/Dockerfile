# Browser service of remotehub (ADR 0007): Chromium on Xvnc displays, one per
# session, started by the agent remotehub-browser on port 4823. Build from the
# repository root:
#
#   docker build -f deploy/browser/Dockerfile -t remotehub-browser .
#
# Run it read-only with a tmpfs on /tmp, and with seccomp:unconfined so that
# Chromium can build its sandbox (the ops package's compose file does both).

# Keep in sync with rust-toolchain.toml (the CI job `base` checks it).
FROM rust:1.98.1-trixie AS agent
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --package remotehub-browser \
 && cp target/release/remotehub-browser /remotehub-browser

FROM debian:trixie-slim
ARG VERSION
LABEL org.opencontainers.image.title="remotehub-browser" \
      org.opencontainers.image.description="Isolated browser for web interfaces of devices, for remotehub" \
      org.opencontainers.image.source="https://github.com/hilman2/remotehub" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.version="${VERSION}"
RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
       chromium fonts-dejavu-core fonts-liberation procps tigervnc-standalone-server tigervnc-tools tini \
  && rm -rf /var/lib/apt/lists/*
# No password manager, downloads, printing, extensions or local files. The
# developer tools are not turned off here: DeveloperToolsAvailability would
# also refuse the agent's DevTools pipe. Kiosk mode keeps them from the user.
COPY deploy/browser/policy.json /etc/chromium/policies/managed/remotehub.json
COPY --from=agent /remotehub-browser /usr/local/bin/remotehub-browser
RUN useradd --uid 10001 --no-create-home --home-dir /tmp browser
USER 10001:10001
EXPOSE 4823
# Renderers of a killed Chromium become children of PID 1: tini reaps them.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/remotehub-browser"]
