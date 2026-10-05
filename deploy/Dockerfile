# Production image of remotehub: the server binary, the built web UI, and
# what it serves under /downloads: the site connector for Windows (#188) and
# the browser extension (#201). Nothing else: no shell, no package manager;
# runs as the unprivileged user 65532 and needs no writable file system.
# Build from the repository root:
#
#   docker build -f deploy/Dockerfile --build-arg VERSION=0.1.0 -t remotehub .
#
# The extension is signed with the key given as the secret `extension_key`
# (release.sh: --secret id=extension_key,src=<PEM file>); without it, with
# the development key in web/, whose ID only development and the CI allow.
#
# The ops package (deploy/ops) runs it with PostgreSQL and guacd.

# Keep in sync with deploy/dev/web.Dockerfile (the CI job `base` checks it).
FROM node:24.21.0-trixie-slim AS web
RUN npm install -g pnpm@12.6.0
WORKDIR /web
COPY web/package.json web/pnpm-lock.yaml web/pnpm-workspace.yaml ./
# The lockfile's packages passed minimumReleaseAge when they were added.
RUN pnpm install --frozen-lockfile --config.minimum-release-age=0
COPY web/ ./
# SvelteKit's version name: fixed per release instead of a timestamp (#49).
ARG VERSION
RUN REMOTEHUB_WEB_VERSION="${VERSION}" pnpm build
# After the UI's build, which compiled the messages the extension uses too.
RUN --mount=type=secret,id=extension_key \
    if [ -s /run/secrets/extension_key ]; then export REMOTEHUB_EXTENSION_KEY=/run/secrets/extension_key; fi \
 && REMOTEHUB_EXTENSION_VERSION="${VERSION}" pnpm build:extension \
 && node scripts/pack-extension.mjs build-extension /downloads

# Keep in sync with rust-toolchain.toml (the CI job `base` checks it).
FROM rust:1.98.1-trixie AS server
# The site connector for Windows (#166) is cross-built with mingw-w64.
# Over HTTPS: a plain-HTTP download of mingw-w64 can stall on the way (#255).
RUN sed -i 's|http://deb.debian.org|https://deb.debian.org|g' /etc/apt/sources.list.d/debian.sources \
  && apt-get update \
  && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 \
  && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY migrations migrations
# The caches only speed up rebuilds on the same machine; the programs are
# copied out of them, since a cache mount is gone after this step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --package remotehub-server \
 && cargo build --release --locked --target x86_64-pc-windows-gnu --package remotehub-connector \
 && cp target/release/remotehub /remotehub \
 && cp target/x86_64-pc-windows-gnu/release/remotehub-connector.exe /remotehub-connector.exe

# The C runtime (glibc, libgcc) and CA certificates for LDAPS, as user 65532.
FROM gcr.io/distroless/cc-debian13:nonroot
ARG VERSION
LABEL org.opencontainers.image.title="remotehub" \
      org.opencontainers.image.description="Browser-based remote access manager and credential vault" \
      org.opencontainers.image.source="https://github.com/hilman2/remotehub" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=server /remotehub /usr/local/bin/remotehub
COPY --from=web /web/build /usr/share/remotehub/web
# The release publishes this very file, taken from the image (release.sh).
COPY --from=server /remotehub-connector.exe /usr/share/remotehub/downloads/remotehub-connector.exe
COPY --from=web /downloads/ /usr/share/remotehub/downloads/
ENV REMOTEHUB_WEB_DIR=/usr/share/remotehub/web \
    REMOTEHUB_DOWNLOADS=/usr/share/remotehub/downloads
USER 65532:65532
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=6s --start-period=60s --retries=3 \
  CMD ["/usr/local/bin/remotehub", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/remotehub"]
