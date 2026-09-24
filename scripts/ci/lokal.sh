#!/usr/bin/env bash
# Local CI for remotehub — the repository's only CI (no Actions workflows).
# Usage and options: bash scripts/ci/lokal.sh --help
#
# gemeinsam.sh is the shared scaffold, kept verbatim across all of the
# maintainer's repositories (German on purpose); only this file is specific
# to remotehub.

CI_REPO_KURZ="remotehub"
CI_JOBS=(base rust)

# shellcheck source=scripts/ci/gemeinsam.sh
source "$(dirname "${BASH_SOURCE[0]}")/gemeinsam.sh"

# Keep in sync with deploy/compose.dev.yml.
POSTGRES_IMAGE="postgres:18.6-trixie"

# CI scripts, line endings, compose files and the same Rust version everywhere.
job_base() {
  local tools
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  ci_docker_run "$tools" bash -euo pipefail -c '
    echo "── ShellCheck"
    shellcheck --version | head -2
    shellcheck -x -P SCRIPTDIR -S warning scripts/ci/*.sh

    echo "── Line endings"
    if grep -rl $'"'"'\r'"'"' scripts/ci deploy; then
      echo "CRLF in scripts/ci or deploy"
      exit 1
    fi

    echo "── Compose (development)"
    docker compose -f deploy/compose.dev.yml --profile workbench config --quiet

    echo "── Rust version"
    wanted="$(sed -n "s/^channel = \"\(.*\)\"/\1/p" rust-toolchain.toml)"
    echo "rust-toolchain.toml: ${wanted}"
    for file in scripts/ci/tools.Dockerfile deploy/dev/rust.Dockerfile; do
      grep -q "^FROM rust:${wanted}-" "$file" || {
        echo "${file} does not use rust:${wanted}"
        exit 1
      }
    done
  '
}

# Formatting, Clippy without warnings and the tests of the whole workspace
# against a fresh PostgreSQL (#[sqlx::test] creates a database per test).
# Every run unpacks the commit afresh; so Rust does not build cold each time,
# the registry and target/ live in volumes without a label — gemeinsam.sh
# removes labelled volumes after the run. Build jobs are capped because the
# Docker host has little memory.
job_rust() {
  local tools
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  ci_dienst db -e POSTGRES_USER=ci -e POSTGRES_PASSWORD=ci -e POSTGRES_DB=ci "$POSTGRES_IMAGE"
  ci_warten db 60 pg_isready -h 127.0.0.1 -U ci -d ci
  ci_docker_run \
    -v remotehub-ci-cargo-registry:/usr/local/cargo/registry \
    -v remotehub-ci-cargo-git:/usr/local/cargo/git \
    -v remotehub-ci-target:/ci-target \
    -e CARGO_TARGET_DIR=/ci-target -e CARGO_BUILD_JOBS=6 -e CARGO_TERM_COLOR=never \
    -e DATABASE_URL=postgres://ci:ci@db:5432/ci \
    "$tools" bash -euo pipefail -c '
      echo "── Toolchain"
      rustc --version
      cargo nextest --version | head -1

      echo "── cargo fmt"
      cargo fmt --all --check

      echo "── cargo clippy"
      cargo clippy --workspace --all-targets --locked -- -D warnings

      echo "── cargo nextest"
      cargo nextest run --workspace --locked --no-tests=warn
    '
}

ci_main "$@"
