# Tool image of the local CI (scripts/ci/lokal.sh): Rust toolchain with
# rustfmt, clippy and cargo-nextest, Node with pnpm for web/, the Docker CLI
# with Compose and Buildx (talks to the host's Docker through the socket) and
# ShellCheck.
FROM docker:29-cli AS docker
# Keep in sync with deploy/dev/web.Dockerfile.
FROM node:24.21.0-trixie-slim AS node

# Keep in sync with rust-toolchain.toml.
FROM rust:1.98.1-trixie
COPY --from=docker /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker /usr/local/libexec/docker/cli-plugins /usr/local/libexec/docker/cli-plugins
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
  && npm install -g pnpm@12.6.0 \
  && node --version && pnpm --version
# mingw-w64: the C compiler for ring's code when clippy checks the connector
# for Windows (#166).
RUN apt-get update \
  && apt-get install -y --no-install-recommends shellcheck mold rsync gcc-mingw-w64-x86-64 \
  && rm -rf /var/lib/apt/lists/*
RUN rustup component add rustfmt clippy \
  && rustup target add x86_64-pc-windows-gnu
# mold links test binaries several times faster than GNU ld; set only here,
# so builds outside these containers are not affected.
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-fuse-ld=mold"
ARG NEXTEST_VERSION=0.9.146
RUN curl -fsSL "https://get.nexte.st/${NEXTEST_VERSION}/linux" | tar -xz -C /usr/local/cargo/bin \
  && cargo nextest --version
