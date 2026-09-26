# Rust container of the development environment (deploy/compose.dev.yml):
# toolchain with rustfmt and clippy, cargo-nextest, and watchexec to rebuild on
# changes. On Windows, file events from bind mounts do not reach containers,
# so watchexec runs with --poll.
# Keep in sync with rust-toolchain.toml and scripts/ci/tools.Dockerfile.
FROM rust:1.98.1-trixie
# The Windows target and mingw-w64 check the site connector for Windows:
# cargo clippy -p remotehub-connector --target x86_64-pc-windows-gnu (#166).
RUN rustup component add rustfmt clippy \
  && rustup target add x86_64-pc-windows-gnu
RUN apt-get update \
  && apt-get install -y --no-install-recommends mold gcc-mingw-w64-x86-64 \
  && rm -rf /var/lib/apt/lists/*
# mold links test binaries several times faster than GNU ld; set only here,
# so builds outside these containers are not affected.
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-fuse-ld=mold"
ARG NEXTEST_VERSION=0.9.146
ARG WATCHEXEC_VERSION=2.7.3
RUN curl -fsSL "https://get.nexte.st/${NEXTEST_VERSION}/linux" | tar -xz -C /usr/local/cargo/bin \
  && curl -fsSL "https://github.com/watchexec/watchexec/releases/download/v${WATCHEXEC_VERSION}/watchexec-${WATCHEXEC_VERSION}-x86_64-unknown-linux-musl.tar.xz" \
     | tar -xJ --strip-components=1 -C /usr/local/bin "watchexec-${WATCHEXEC_VERSION}-x86_64-unknown-linux-musl/watchexec" \
  && cargo nextest --version && watchexec --version
ENV TZ=Europe/Berlin CARGO_TARGET_DIR=/target
WORKDIR /work
