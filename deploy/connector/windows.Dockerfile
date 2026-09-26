# The site connector for Windows (#166): remotehub-connector.exe, built with
# mingw-w64 for x86_64-pc-windows-gnu. From the repository root:
#
#   docker build -f deploy/connector/windows.Dockerfile --output type=local,dest=out .
#
# writes out/remotehub-connector.exe. docs/install.md, "On a Windows server",
# shows how to install it.

# Keep in sync with rust-toolchain.toml (the CI job `base` checks it).
FROM rust:1.98.1-trixie AS build
RUN apt-get update \
  && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 \
  && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
# The caches only speed up rebuilds on the same machine; the program is
# copied out of them, since a cache mount is gone after this step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --target x86_64-pc-windows-gnu --package remotehub-connector \
 && cp target/x86_64-pc-windows-gnu/release/remotehub-connector.exe /remotehub-connector.exe

FROM scratch
COPY --from=build /remotehub-connector.exe /remotehub-connector.exe
