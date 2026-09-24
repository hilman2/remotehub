# Production image of remotehub: the server binary and the built web UI,
# nothing else. No shell, no package manager; runs as the unprivileged user
# 65532 and needs no writable file system. Build from the repository root:
#
#   docker build -f deploy/Dockerfile --build-arg VERSION=0.1.0 -t remotehub .
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

# Keep in sync with rust-toolchain.toml (the CI job `base` checks it).
FROM rust:1.98.1-trixie AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY migrations migrations
# The caches only speed up rebuilds on the same machine; the binary is
# copied out of them, since a cache mount is gone after this step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --package remotehub-server \
 && cp target/release/remotehub /remotehub

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
ENV REMOTEHUB_WEB_DIR=/usr/share/remotehub/web
USER 65532:65532
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=6s --start-period=60s --retries=3 \
  CMD ["/usr/local/bin/remotehub", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/remotehub"]
