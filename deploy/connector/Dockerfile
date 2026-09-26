# Image of the site connector (ADR 0008, ADR 0011): the binary
# remotehub-connector, nothing else. No shell, no package manager; runs as
# the unprivileged user 65532 with a read-only file system, and writes only
# to its data directory, a volume. Build from the repository root:
#
#   docker build -f deploy/connector/Dockerfile --build-arg VERSION=0.1.0 -t remotehub-connector .
#
# docs/install.md, "Reach devices in other networks", shows how to run it.

# Keep in sync with rust-toolchain.toml (the CI job `base` checks it).
FROM rust:1.98.1-trixie AS connector
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
# The caches only speed up rebuilds on the same machine; the binary is
# copied out of them, since a cache mount is gone after this step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --package remotehub-connector \
 && cp target/release/remotehub-connector /remotehub-connector \
 && mkdir /data

# The C runtime (glibc, libgcc) and CA certificates for remotehub's HTTPS,
# as user 65532.
FROM gcr.io/distroless/cc-debian13:nonroot
ARG VERSION
LABEL org.opencontainers.image.title="remotehub-connector" \
      org.opencontainers.image.description="remotehub site connector: reaches devices in a network remotehub cannot reach" \
      org.opencontainers.image.source="https://github.com/hilman2/remotehub" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=connector /remotehub-connector /usr/local/bin/remotehub-connector
# A named volume mounted here takes over the owner 65532. No VOLUME line:
# a run without one would leave an anonymous volume behind.
COPY --from=connector --chown=65532:65532 /data /var/lib/remotehub-connector
# The published port decides who reaches the web interface.
ENV REMOTEHUB_CONNECTOR_LISTEN=0.0.0.0:8480
EXPOSE 8480
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/remotehub-connector"]
