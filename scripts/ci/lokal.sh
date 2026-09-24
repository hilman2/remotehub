#!/usr/bin/env bash
# Local CI for remotehub — the repository's only CI (no Actions workflows).
# Usage and options: bash scripts/ci/lokal.sh --help
#
# gemeinsam.sh is the scaffold (lock, commit status, containers); it started
# as a copy from another repository and belongs to remotehub alone now.
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

# Keep in sync with deploy/compose.dev.yml (and web/package.json for Playwright).
POSTGRES_IMAGE="postgres:18.6-trixie"
E2E_IMAGE="mcr.microsoft.com/playwright:v1.63.0-noble"

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

# Runs a script in the Rust tools container with the cargo caches, a fresh
# PostgreSQL (#[sqlx::test] creates a database per test) and the lab.
#
# Only what changed is compiled: every run unpacks the commit into a new
# volume under a new path, which cargo would treat as new packages. So the
# source is synced into a volume at the fixed path /src, by content: only
# files that really changed get written (and a new mtime), and cargo rebuilds
# just the crates they belong to. The registry, target/ and /src live in
# volumes without a label — gemeinsam.sh removes labelled volumes after a run.
cargo_run() { # tools script
  ci_docker_run \
    -v remotehub-ci-cargo-registry:/usr/local/cargo/registry \
    -v remotehub-ci-cargo-git:/usr/local/cargo/git \
    -v remotehub-ci-target:/ci-target \
    -v remotehub-ci-src:/src \
    -e CARGO_TARGET_DIR=/ci-target -e CARGO_BUILD_JOBS=8 -e CARGO_TERM_COLOR=never \
    -e DATABASE_URL=postgres://ci:ci@db:5432/ci \
    -e REMOTEHUB_TEST_LDAP_URL=ldaps://dc.remotehub.test \
    -e REMOTEHUB_TEST_SSH_HOST=ssh-target \
    "$1" bash -euo pipefail -c "
      rsync -rlc --delete \\
        --exclude=/web/node_modules/ --exclude=/web/.svelte-kit/ \\
        --exclude=/web/build/ --exclude=/web/src/lib/paraglide/ \\
        ./ /src/
      cd /src
      $2"
}

# Rust: formatting, Clippy without warnings and the tests of the whole
# workspace; if the lab is needed, it starts while Rust compiles, and the
# tests marked #[ignore = "needs the test lab"] run afterwards with the test
# binaries just built.
part_rust() { # tools lab(0|1)
  local tools="$1" lab="$2" lab_pid=""
  if [ "$lab" = 1 ]; then
    start_lab "$tools" &
    lab_pid=$!
  fi
  ci_warten db 60 pg_isready -h 127.0.0.1 -U ci -d ci
  cargo_run "$tools" '
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
  if [ -n "$lab_pid" ]; then
    wait "$lab_pid"
    cargo_run "$tools" '
      echo "── cargo nextest (test lab)"
      cargo nextest run --workspace --locked --no-tests=warn --run-ignored only
    '
  fi
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
# then all four checks run in parallel — vitest and the build leave the
# compiled messages alone (PARAGLIDE_PRECOMPILED, see web/vite.config.ts).
# The lockfile's packages were checked against minimumReleaseAge when they
# were added, so the install does not ask the registry again.
# The pnpm store stays between runs in a volume without a label.
part_web() { # tools
  ci_docker_run \
    -v remotehub-ci-pnpm-store:/pnpm-store \
    -e pnpm_config_store_dir=/pnpm-store \
    -e PARAGLIDE_PRECOMPILED=1 \
    "$1" bash -euo pipefail -c '
      cd web
      pnpm install --frozen-lockfile --config.minimum-release-age=0
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
      step build pnpm exec vite build &
      wait

      failed=0
      for name in svelte-check lint vitest build; do
        echo "── $name: $(cat "/tmp/$name.result")"
        cat "/tmp/$name.log"
        echo
        grep -q "^ok" "/tmp/$name.result" || failed=1
      done
      [ "$failed" = 0 ]
    '
}

# End-to-end tests in a real browser (web/tests/e2e) with the server binary
# and the UI built in this run, a fresh database and the lab. The browser
# shares the server's network namespace, so it reaches it as localhost —
# Secure cookies need localhost or HTTPS.
part_e2e() { # tools
  ci_dienst db-e2e -e POSTGRES_USER=ci -e POSTGRES_PASSWORD=ci -e POSTGRES_DB=e2e "$POSTGRES_IMAGE"
  ci_warten db-e2e 60 pg_isready -h 127.0.0.1 -U ci -d e2e
  ci_dienst e2e-server \
    -v remotehub-ci-target:/ci-target -v "${CI_VOLUME}:${CI_SRC}" \
    -e REMOTEHUB_DATABASE_URL=postgres://ci:ci@db-e2e:5432/e2e \
    -e REMOTEHUB_PUBLIC_URL=http://localhost:8080 \
    -e REMOTEHUB_MASTER_KEY_FILE="${CI_SRC}/deploy/dev/master.key" \
    -e REMOTEHUB_WEB_DIR="${CI_SRC}/web/build" \
    -e REMOTEHUB_LDAP_URL=ldaps://dc.remotehub.test \
    -e REMOTEHUB_LDAP_CA_FILE="${CI_SRC}/deploy/testlab/dc/tls/ca.crt" \
    -e REMOTEHUB_LDAP_BIND_DN=svc-remotehub@remotehub.test \
    -e 'REMOTEHUB_LDAP_BIND_PASSWORD=Svc-Passw0rd!' \
    -e REMOTEHUB_LDAP_BASE_DN=DC=remotehub,DC=test \
    -e 'REMOTEHUB_ADMIN_GROUPS=RH Admins' \
    "$1" /ci-target/debug/remotehub
  ci_warten e2e-server 60 bash -c '</dev/tcp/127.0.0.1/8080'
  docker run --rm --label "ci-lokal=${CI_ID}" --network "container:${CI_ID}-e2e-server" \
    -v "${CI_VOLUME}:${CI_SRC}" -w "${CI_SRC}/web" \
    -e E2E_BASE_URL=http://localhost:8080 -e E2E_SSH_HOST=ssh-target -e CI=1 \
    "$E2E_IMAGE" node node_modules/@playwright/test/cli.js test
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

# Rust (with the lab) and web in parallel, each with its own log; then, in
# full runs (or with CI_E2E=1), the end-to-end tests with what both built.
# Failed parts are printed last so the summary of gemeinsam.sh shows them.
job_code() {
  local tools logs rust=0 web=0 lab=0 rust_pid="" web_pid="" e2e_pid="" rust_rc=0 web_rc=0 e2e_rc=0
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  logs="$(mktemp -d)"
  if needed "${RUST_INPUTS[@]}"; then rust=1; fi
  if needed "${LAB_INPUTS[@]}"; then lab=1; rust=1; fi
  if needed "${WEB_INPUTS[@]}"; then web=1; fi

  if [ "$rust" = 1 ]; then
    ci_dienst db -e POSTGRES_USER=ci -e POSTGRES_PASSWORD=ci -e POSTGRES_DB=ci "$POSTGRES_IMAGE"
    in_background rust part_rust "$tools" "$lab"
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

  # Full runs (and changes under scripts/ci/ or to the tests themselves)
  # include them; CI_E2E=1 forces them.
  local full=0
  if [ "${CI_E2E:-0}" = 1 ] || needed web/tests/e2e/ web/playwright.config.ts; then full=1; fi
  if [ "$full" = 1 ] && [ "$lab" = 1 ] && [ "$web" = 1 ] && [ "$rust_rc" = 0 ] && [ "$web_rc" = 0 ]; then
    in_background e2e part_e2e "$tools"
    wait "$e2e_pid" || e2e_rc=$?
  else
    echo "skipped: end-to-end tests run in full runs after Rust and web passed (CI_E2E=1 forces them)" >"${logs}/e2e.log"
  fi

  # Successful parts first, failed parts last.
  local part rc seconds
  for want in ok failed; do
    for part in rust web e2e; do
      rc="${part}_rc"
      seconds="$(cat "${logs}/${part}.seconds" 2>/dev/null || echo 0)"
      if [ "$want" = ok ] && [ "${!rc}" = 0 ]; then
        printf '════ %s ✓ %ss\n' "$part" "$seconds"
        cat "${logs}/${part}.log"
      elif [ "$want" = failed ] && [ "${!rc}" != 0 ]; then
        printf '════ %s ✗ %ss (exit %s)\n' "$part" "$seconds" "${!rc}"
        cat "${logs}/${part}.log"
      fi
    done
  done
  rm -rf "$logs"
  [ "$rust_rc" = 0 ] && [ "$web_rc" = 0 ] && [ "$e2e_rc" = 0 ]
}

ci_main "$@"
