#!/usr/bin/env bash
# Local CI for remotehub — the repository's only CI (no Actions workflows).
# Usage and options: bash scripts/ci/lokal.sh --help
#
# gemeinsam.sh is the shared scaffold, kept verbatim across all of the
# maintainer's repositories (German on purpose); only this file is specific
# to remotehub.

CI_REPO_KURZ="remotehub"
CI_JOBS=(base rust web integration)

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
# Tests that need the test lab are #[ignore]d here and run in the job
# integration.
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

# User interface: types, formatting and lint, tests (including the
# translation guards) and the static build. The pnpm store stays between runs
# in a volume without a label.
job_web() {
  local tools
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  ci_docker_run \
    -v remotehub-ci-pnpm-store:/pnpm-store \
    -e pnpm_config_store_dir=/pnpm-store \
    "$tools" bash -euo pipefail -c '
      cd web
      pnpm install --frozen-lockfile

      echo "── svelte-check"
      pnpm check

      echo "── prettier and eslint"
      pnpm lint

      echo "── vitest"
      pnpm test

      echo "── build"
      pnpm build
    '
}

# Integration tests against the test lab (deploy/testlab): a Samba AD domain
# controller and an SSH target, built from the commit under test (the build
# cache keeps this fast), plus a fresh PostgreSQL. Runs exactly the tests
# marked #[ignore = "needs the test lab"].
job_integration() {
  local tools
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  ci_docker_run "$tools" bash -euo pipefail -c '
    docker build --quiet --tag remotehub-ci-testlab-dc deploy/testlab/dc
    docker build --quiet --tag remotehub-ci-testlab-ssh deploy/testlab/ssh
  '
  ci_dienst db-integration -e POSTGRES_USER=ci -e POSTGRES_PASSWORD=ci -e POSTGRES_DB=ci "$POSTGRES_IMAGE"
  ci_dienst dc --hostname dc --network-alias dc.remotehub.test remotehub-ci-testlab-dc
  ci_dienst ssh-target --hostname ssh-target remotehub-ci-testlab-ssh
  ci_warten db-integration 60 pg_isready -h 127.0.0.1 -U ci -d ci
  ci_warten dc 60 bash -c '</dev/tcp/127.0.0.1/636'
  ci_warten ssh-target 30 bash -c '</dev/tcp/127.0.0.1/22'
  ci_docker_run \
    -v remotehub-ci-cargo-registry:/usr/local/cargo/registry \
    -v remotehub-ci-cargo-git:/usr/local/cargo/git \
    -v remotehub-ci-target:/ci-target \
    -e CARGO_TARGET_DIR=/ci-target -e CARGO_BUILD_JOBS=6 -e CARGO_TERM_COLOR=never \
    -e DATABASE_URL=postgres://ci:ci@db-integration:5432/ci \
    -e REMOTEHUB_TEST_LDAP_URL=ldaps://dc.remotehub.test \
    -e REMOTEHUB_TEST_SSH_HOST=ssh-target \
    "$tools" bash -euo pipefail -c '
      echo "── cargo nextest (integration)"
      cargo nextest run --workspace --locked --no-tests=warn --run-ignored only
    '
}

ci_main "$@"
