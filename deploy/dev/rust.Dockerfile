# Rust container of the development environment (deploy/compose.dev.yml):
# toolchain with rustfmt and clippy, cargo-nextest, and watchexec to rebuild on
# changes. On Windows, file events from bind mounts do not reach containers,
# so watchexec runs with --poll.
# Keep in sync with rust-toolchain.toml and scripts/ci/tools.Dockerfile.
FROM rust:1.98.1-trixie
RUN rustup component add rustfmt clippy
ARG NEXTEST_VERSION=0.9.146
ARG WATCHEXEC_VERSION=2.7.3
RUN curl -fsSL "https://get.nexte.st/${NEXTEST_VERSION}/linux" | tar -xz -C /usr/local/cargo/bin \
  && curl -fsSL "https://github.com/watchexec/watchexec/releases/download/v${WATCHEXEC_VERSION}/watchexec-${WATCHEXEC_VERSION}-x86_64-unknown-linux-musl.tar.xz" \
     | tar -xJ --strip-components=1 -C /usr/local/bin "watchexec-${WATCHEXEC_VERSION}-x86_64-unknown-linux-musl/watchexec" \
  && cargo nextest --version && watchexec --version
ENV TZ=Europe/Berlin CARGO_TARGET_DIR=/target
WORKDIR /work
