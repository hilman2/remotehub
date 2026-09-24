#!/usr/bin/env bash
# Local CI for remotehub — the repository's only CI (no Actions workflows).
# Usage and options: bash scripts/ci/lokal.sh --help
#
# gemeinsam.sh is the shared scaffold, kept verbatim across all of the
# maintainer's repositories (German on purpose); only this file is specific
# to remotehub.
#
# Fast by design:
# - Only what a change can affect runs: the files changed since the merge
#   base with main decide (see INPUTS below). A commit on main itself, a
#   change to scripts/ci/ or CI_FULL=1 runs everything.
# - Rust and the web UI are checked in parallel; the test lab starts while
#   Rust compiles, and its tests reuse the test binaries just built.

CI_REPO_KURZ="remotehub"
CI_JOBS=(base code)

# shellcheck source=scripts/ci/gemeinsam.sh
source "$(dirname "${BASH_SOURCE[0]}")/gemeinsam.sh"

# Keep in sync with deploy/compose.dev.yml.
POSTGRES_IMAGE="postgres:18.6-trixie"

# What each part depends on (path prefixes). scripts/ci/ counts for all.
RUST_INPUTS=(crates/ migrations/ Cargo.toml Cargo.lock rust-toolchain.toml deploy/dev/rust.Dockerfile)
WEB_INPUTS=(web/ deploy/dev/web.Dockerfile)
LAB_INPUTS=("${RUST_INPUTS[@]}" deploy/testlab/)

# Files changed between the merge base with main and the commit under test.
# Fails when everything has to run: CI_FULL=1, no merge base, or a commit
# that is already on main (e.g. before a release).
changed_files() {
  [ "${CI_FULL:-0}" != 1 ] || return 1
  git -C "$CI_WURZEL" fetch -q origin main 2>/dev/null || true
  local base
  base="$(git -C "$CI_WURZEL" merge-base origin/main "$CI_SHA" 2>/dev/null)" || return 1
  [ "$base" != "$CI_SHA" ] || return 1
  git -C "$CI_WURZEL" diff --name-only "$base" "$CI_SHA"
}

# Whether a part has to run for the given inputs.
needed() {
  local files input
  files="$(changed_files)" || return 0
  for input in "$@" scripts/ci/; do
    if grep -q "^${input}" <<<"$files"; then
      return 0
    fi
  done
  return 1
}

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

# Rust: formatting, Clippy without warnings, the tests of the whole workspace
# against a fresh PostgreSQL (#[sqlx::test] creates a database per test) and,
# if the lab is needed, the tests marked #[ignore = "needs the test lab"]
# against the Samba domain controller and the SSH target.
# Every run unpacks the commit afresh; so Rust does not build cold each time,
# the registry and target/ live in volumes without a label — gemeinsam.sh
# removes labelled volumes after the run. Build jobs are capped because the
# Docker host has little memory.
part_rust() { # tools lab(0|1)
  local tools="$1" lab="$2"
  ci_warten db 60 pg_isready -h 127.0.0.1 -U ci -d ci
  ci_docker_run \
    -v remotehub-ci-cargo-registry:/usr/local/cargo/registry \
    -v remotehub-ci-cargo-git:/usr/local/cargo/git \
    -v remotehub-ci-target:/ci-target \
    -e CARGO_TARGET_DIR=/ci-target -e CARGO_BUILD_JOBS=6 -e CARGO_TERM_COLOR=never \
    -e DATABASE_URL=postgres://ci:ci@db:5432/ci \
    -e REMOTEHUB_TEST_LDAP_URL=ldaps://dc.remotehub.test \
    -e REMOTEHUB_TEST_SSH_HOST=ssh-target \
    -e LAB="$lab" \
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

      if [ "$LAB" = 1 ]; then
        echo "── cargo nextest (test lab)"
        cargo nextest run --workspace --locked --no-tests=warn --run-ignored only
      fi
    '
}

# Starts the lab (images from the commit under test; the build cache keeps
# this fast) and waits until it answers.
start_lab() { # tools
  ci_docker_run "$1" bash -euo pipefail -c '
    docker build --quiet --tag remotehub-ci-testlab-dc deploy/testlab/dc
    docker build --quiet --tag remotehub-ci-testlab-ssh deploy/testlab/ssh
  ' >/dev/null
  ci_dienst dc --hostname dc --network-alias dc.remotehub.test remotehub-ci-testlab-dc
  ci_dienst ssh-target --hostname ssh-target remotehub-ci-testlab-ssh
  ci_warten dc 60 bash -c '</dev/tcp/127.0.0.1/636'
  ci_warten ssh-target 30 bash -c '</dev/tcp/127.0.0.1/22'
}

# User interface: types, formatting and lint, tests (including the
# translation guards) and the static build. The messages are compiled once;
# then svelte-check, prettier/eslint and vitest run in parallel (they only
# read), and the build comes last because it writes the messages again.
# The pnpm store stays between runs in a volume without a label.
part_web() { # tools
  ci_docker_run \
    -v remotehub-ci-pnpm-store:/pnpm-store \
    -e pnpm_config_store_dir=/pnpm-store \
    "$1" bash -euo pipefail -c '
      cd web
      pnpm install --frozen-lockfile
      pnpm i18n
      pnpm exec svelte-kit sync

      step() { # name command...
        local name="$1" start=$SECONDS
        shift
        if "$@" >"/tmp/$name.log" 2>&1; then
          echo "ok $((SECONDS - start))s" >"/tmp/$name.result"
        else
          echo "FAILED $((SECONDS - start))s" >"/tmp/$name.result"
        fi
      }
      step svelte-check pnpm exec svelte-check --tsconfig ./tsconfig.json --fail-on-warnings &
      step lint pnpm lint &
      step vitest pnpm exec vitest --run &
      wait

      failed=0
      for name in svelte-check lint vitest; do
        echo "── $name: $(cat "/tmp/$name.result")"
        cat "/tmp/$name.log"
        grep -q "^ok" "/tmp/$name.result" || failed=1
      done
      [ "$failed" = 0 ]

      echo "── build"
      pnpm build
    '
}

# Runs a part in the background with its own log and records how long it took.
#   in_background NAME COMMAND... (sets NAME_pid)
in_background() {
  local name="$1"
  shift
  (
    local start=$SECONDS rc=0
    # In the background, not in an || list, so set -e keeps working inside.
    "$@" &
    wait "$!" || rc=$?
    echo "$((SECONDS - start))" >"${logs}/${name}.seconds"
    exit "$rc"
  ) >"${logs}/${name}.log" 2>&1 &
  printf -v "${name}_pid" "%s" "$!"
}

rust_with_lab() { # tools lab
  if [ "$2" = 1 ]; then start_lab "$1"; fi
  part_rust "$1" "$2"
}

# Rust (with the lab) and web in parallel, each with its own log; failed parts
# are printed last so the summary of gemeinsam.sh shows them.
job_code() {
  local tools logs rust=0 web=0 lab=0 rust_pid="" web_pid="" rust_rc=0 web_rc=0
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  logs="$(mktemp -d)"
  if needed "${RUST_INPUTS[@]}"; then rust=1; fi
  if needed "${LAB_INPUTS[@]}"; then lab=1; rust=1; fi
  if needed "${WEB_INPUTS[@]}"; then web=1; fi

  if [ "$rust" = 1 ]; then
    ci_dienst db -e POSTGRES_USER=ci -e POSTGRES_PASSWORD=ci -e POSTGRES_DB=ci "$POSTGRES_IMAGE"
    in_background rust rust_with_lab "$tools" "$lab"
  else
    echo "skipped: nothing under ${RUST_INPUTS[*]} deploy/testlab/ changed" >"${logs}/rust.log"
  fi
  if [ "$web" = 1 ]; then
    in_background web part_web "$tools"
  else
    echo "skipped: nothing under ${WEB_INPUTS[*]} changed" >"${logs}/web.log"
  fi

  if [ -n "$rust_pid" ]; then wait "$rust_pid" || rust_rc=$?; fi
  if [ -n "$web_pid" ]; then wait "$web_pid" || web_rc=$?; fi

  # Successful parts first, failed parts last.
  local part rc seconds
  for want in ok failed; do
    for part in rust web; do
      rc="${part}_rc"
      seconds="$(cat "${logs}/${part}.seconds" 2>/dev/null || echo 0)"
      if [ "$want" = ok ] && [ "${!rc}" = 0 ]; then
        printf '════ %s ✓ %ss
' "$part" "$seconds"
        cat "${logs}/${part}.log"
      elif [ "$want" = failed ] && [ "${!rc}" != 0 ]; then
        printf '════ %s ✗ %ss (exit %s)
' "$part" "$seconds" "${!rc}"
        cat "${logs}/${part}.log"
      fi
    done
  done
  rm -rf "$logs"
  [ "$rust_rc" = 0 ] && [ "$web_rc" = 0 ]
}

ci_main "$@"
