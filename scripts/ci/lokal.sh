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
CI_JOBS=(base code image)

# shellcheck source=scripts/ci/gemeinsam.sh
source "$(dirname "${BASH_SOURCE[0]}")/gemeinsam.sh"

# Keep in sync with deploy/compose.dev.yml and deploy/ops/compose.yml (the job
# `base` checks both), and web/package.json for Playwright.
POSTGRES_IMAGE="postgres:18.6-trixie"
E2E_IMAGE="mcr.microsoft.com/playwright:v1.63.0-noble"
# Keep equal to deploy/compose.dev.yml and deploy/ops/compose.yml.
KRATOS_IMAGE="oryd/kratos:v26.2.0"
# The lab's OpenID Connect provider (#109); keep equal to deploy/compose.dev.yml.
DEX_IMAGE="ghcr.io/dexidp/dex:v2.43.1"

# What each part depends on (path prefixes). scripts/ci/ counts for all.
RUST_INPUTS=(crates/ migrations/ Cargo.toml Cargo.lock rust-toolchain.toml deploy/dev/rust.Dockerfile)
WEB_INPUTS=(web/ deploy/dev/web.Dockerfile)
LAB_INPUTS=("${RUST_INPUTS[@]}" deploy/testlab/ deploy/guacd/ deploy/browser/)
# The end-to-end tests check the UI most; CI_E2E=1 forces them.
E2E_INPUTS=(web/src/ web/tests/e2e/ web/playwright.config.ts deploy/ops/kratos/ deploy/testlab/oidc/)
# The production images build a release binary; they are tried when they or
# the ops package change, and in full runs (every commit on main, releases).
IMAGE_INPUTS=(deploy/Dockerfile .dockerignore deploy/ops/ deploy/guacd/ deploy/browser/)

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

    echo "── Compose (development, ops package)"
    docker compose -f deploy/compose.dev.yml --profile workbench config --quiet
    REMOTEHUB_VERSION=0 REMOTEHUB_PUBLIC_URL=https://x REMOTEHUB_HOST=x \
      docker compose -f deploy/ops/compose.yml config --quiet --no-path-resolution

    echo "── Rust version"
    wanted="$(sed -n "s/^channel = \"\(.*\)\"/\1/p" rust-toolchain.toml)"
    echo "rust-toolchain.toml: ${wanted}"
    for file in scripts/ci/tools.Dockerfile deploy/dev/rust.Dockerfile deploy/Dockerfile deploy/browser/Dockerfile; do
      grep -q "^FROM rust:${wanted}-" "$file" || {
        echo "${file} does not use rust:${wanted}"
        exit 1
      }
    done

    echo "── Node and PostgreSQL versions"
    same() { # what pattern files...
      local what="$1" pattern="$2"
      shift 2
      local found
      found="$(grep -ho "$pattern" "$@" | sort -u)"
      echo "${what}: ${found}"
      [ "$(wc -l <<<"$found")" = 1 ] || {
        echo "$* disagree on ${what}"
        exit 1
      }
    }
    same node "FROM node:[^ ]*" deploy/dev/web.Dockerfile deploy/Dockerfile scripts/ci/tools.Dockerfile
    same pnpm "pnpm@[0-9.]*" deploy/dev/web.Dockerfile deploy/Dockerfile scripts/ci/tools.Dockerfile
    same postgres "postgres:[0-9][^ \"]*" deploy/compose.dev.yml deploy/ops/compose.yml scripts/ci/lokal.sh
  '
}

# The production images (deploy/Dockerfile, deploy/guacd, deploy/browser) with the ops
# package (deploy/ops), tried the way an installation uses them: see
# scripts/ci/image-check.sh, which the release script runs alike.
job_image() {
  if ! needed "${IMAGE_INPUTS[@]}"; then
    echo "skipped: nothing under ${IMAGE_INPUTS[*]} changed"
    return 0
  fi
  local tools version
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  version="$(git -C "$CI_WURZEL" show "${CI_SHA}:Cargo.toml" | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1)"
  ci_docker_run "$tools" bash scripts/ci/image-check.sh "$version" remotehub-ci-image remotehub-ci-guacd \
    remotehub-ci-browser-image ci
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
    -e REMOTEHUB_TEST_GUACD=guacd:4822 \
    -e REMOTEHUB_TEST_DESKTOP_HOST=desktop-target \
    -e REMOTEHUB_TEST_BROWSER=browser:4823 \
    -e REMOTEHUB_TEST_WEB_HOST=web-target \
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

# The server binary for the end-to-end tests, with the lab, when no Rust
# input changed and Rust itself need not be checked.
part_server() { # tools
  local lab_pid
  start_lab "$1" &
  lab_pid=$!
  cargo_run "$1" '
    echo "── cargo build (the server for the end-to-end tests)"
    cargo build --locked -p remotehub-server --bin remotehub
  '
  wait "$lab_pid"
}

# Starts the lab (images from the commit under test; the build cache keeps
# this fast) and waits until it answers.
start_lab() { # tools
  ci_docker_run "$1" bash -euo pipefail -c '
    docker build --quiet --tag remotehub-ci-testlab-dc deploy/testlab/dc
    docker build --quiet --tag remotehub-ci-testlab-ssh deploy/testlab/ssh
    docker build --quiet --tag remotehub-ci-testlab-desktop deploy/testlab/desktop
    docker build --quiet --tag remotehub-ci-testlab-web deploy/testlab/web
    docker build --quiet --tag remotehub-ci-guacd deploy/guacd
    docker build --quiet --tag remotehub-ci-browser --file deploy/browser/Dockerfile .
  ' >/dev/null
  ci_dienst dc --hostname dc --network-alias dc.remotehub.test remotehub-ci-testlab-dc
  ci_dienst ssh-target --hostname ssh-target remotehub-ci-testlab-ssh
  ci_dienst desktop-target --hostname desktop-target remotehub-ci-testlab-desktop
  ci_dienst web-target --hostname web-target remotehub-ci-testlab-web
  ci_dienst guacd --read-only --tmpfs /tmp --tmpfs /home/guacd:uid=1000,mode=0700 \
    --cap-drop ALL --security-opt no-new-privileges remotehub-ci-guacd
  # As deploy/ops/compose.yml runs it: Chromium's sandbox needs seccomp:unconfined.
  ci_dienst browser --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges \
    --security-opt seccomp=unconfined remotehub-ci-browser
  ci_warten dc 60 bash -c '</dev/tcp/127.0.0.1/636'
  ci_warten ssh-target 30 bash -c '</dev/tcp/127.0.0.1/22'
  ci_warten desktop-target 30 bash -c '</dev/tcp/127.0.0.1/3389 && </dev/tcp/127.0.0.1/5900'
  ci_warten web-target 30 bash -c '</dev/tcp/127.0.0.1/443 && </dev/tcp/127.0.0.1/8080'
  ci_warten guacd 30 bash -c '</dev/tcp/127.0.0.1/4822'
  ci_warten browser 30 bash -c '</dev/tcp/127.0.0.1/4823'
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
    -e REMOTEHUB_WEB_VERSION="$CI_SHA" \
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

# Ory Kratos for the local accounts of the end-to-end tests (#103): its own
# database next to e2e's, the configuration of deploy/ops/kratos with the
# addresses of this run. The source volume cannot be mounted where the
# configuration expects its identity schema, so the schema's path is set too.
start_kratos() {
  # The OpenID Connect provider Kratos signs in with (deploy/testlab/oidc).
  ci_dienst oidc -v "${CI_VOLUME}:${CI_SRC}:ro" "$DEX_IMAGE" \
    dex serve "${CI_SRC}/deploy/testlab/oidc/config.yaml"
  docker exec "${CI_ID}-db-e2e" createdb -U ci kratos
  local base=http://localhost:8080
  local config="${CI_SRC}/deploy/ops/kratos/kratos.yml"
  local env=(
    -v "${CI_VOLUME}:${CI_SRC}:ro"
    -e "DSN=postgres://ci:ci@db-e2e:5432/kratos?sslmode=disable"
    -e "SERVE_PUBLIC_BASE_URL=${base}/api/auth/"
    -e "SELFSERVICE_DEFAULT_BROWSER_RETURN_URL=${base}/"
    -e "SELFSERVICE_ALLOWED_RETURN_URLS=[\"${base}/\"]"
    -e "SELFSERVICE_FLOWS_ERROR_UI_URL=${base}/sign-in"
    -e "SELFSERVICE_FLOWS_LOGIN_UI_URL=${base}/sign-in"
    -e "SELFSERVICE_FLOWS_REGISTRATION_UI_URL=${base}/sign-in"
    -e SELFSERVICE_METHODS_WEBAUTHN_CONFIG_RP_ID=localhost
    -e "SELFSERVICE_METHODS_WEBAUTHN_CONFIG_RP_ORIGINS=[\"${base}\"]"
    -e SELFSERVICE_METHODS_PASSKEY_CONFIG_RP_ID=localhost
    -e "SELFSERVICE_METHODS_PASSKEY_CONFIG_RP_ORIGINS=[\"${base}\"]"
    -e "SELFSERVICE_FLOWS_SETTINGS_UI_URL=${base}/account"
    -e "SELFSERVICE_FLOWS_RECOVERY_UI_URL=${base}/sign-in/recovery"
    -e "SELFSERVICE_FLOWS_LOGOUT_AFTER_DEFAULT_BROWSER_RETURN_URL=${base}/sign-in"
    -e "IDENTITY_SCHEMAS=[{\"id\":\"user\",\"url\":\"file://${CI_SRC}/deploy/ops/kratos/identity.schema.json\"}]"
    -e 'SECRETS_COOKIE=["ci-cookie-secret-not-for-production"]'
    -e 'SECRETS_CIPHER=["ci-cipher-secret-32-characters!!"]'
    -e 'SECRETS_DEFAULT=["ci-default-secret-not-for-production"]'
    -e SELFSERVICE_METHODS_OIDC_ENABLED=true
    -e "SELFSERVICE_METHODS_OIDC_CONFIG_PROVIDERS=[{\"id\":\"lab\",\"label\":\"Lab\",\"provider\":\"generic\",\"issuer_url\":\"http://oidc:5556/dex\",\"client_id\":\"remotehub\",\"client_secret\":\"lab-oidc-secret\",\"scope\":[\"openid\",\"email\",\"profile\"],\"mapper_url\":\"file://${CI_SRC}/deploy/ops/kratos/oidc.jsonnet\"}]"
    -e SQA_OPT_OUT=true
    -e LOG_FORMAT=text
  )
  docker run --rm --label "ci-lokal=${CI_ID}" --network "$CI_NETZ" "${env[@]}" \
    "$KRATOS_IMAGE" -c "$config" migrate sql up -e --yes >/dev/null
  ci_dienst kratos "${env[@]}" "$KRATOS_IMAGE" serve -c "$config" --dev
  # The image has no shell: the database's container asks for it.
  ci_warten db-e2e 30 bash -c '</dev/tcp/kratos/4433'
}

# End-to-end tests in a real browser (web/tests/e2e) with the server binary
# and the UI built in this run, a fresh database and the lab. The browser
# shares the server's network namespace, so it reaches it as localhost —
# Secure cookies need localhost or HTTPS.
part_e2e() { # tools
  ci_dienst db-e2e -e POSTGRES_USER=ci -e POSTGRES_PASSWORD=ci -e POSTGRES_DB=e2e "$POSTGRES_IMAGE"
  ci_warten db-e2e 60 pg_isready -h 127.0.0.1 -U ci -d e2e
  start_kratos
  ci_dienst e2e-server \
    -v remotehub-ci-target:/ci-target -v "${CI_VOLUME}:${CI_SRC}" \
    -e REMOTEHUB_DATABASE_URL=postgres://ci:ci@db-e2e:5432/e2e \
    -e REMOTEHUB_PUBLIC_URL=http://localhost:8080 \
    -e REMOTEHUB_MASTER_KEY_FILE="${CI_SRC}/deploy/dev/master.key" \
    -e REMOTEHUB_SSH_CA_KEY_FILE="${CI_SRC}/deploy/testlab/ssh/remotehub_ca" \
    -e REMOTEHUB_WEB_DIR="${CI_SRC}/web/build" \
    -e REMOTEHUB_LDAP_URL=ldaps://dc.remotehub.test \
    -e REMOTEHUB_LDAP_CA_FILE="${CI_SRC}/deploy/testlab/dc/tls/ca.crt" \
    -e REMOTEHUB_LDAP_BIND_DN=svc-remotehub@remotehub.test \
    -e 'REMOTEHUB_LDAP_BIND_PASSWORD=Svc-Passw0rd!' \
    -e REMOTEHUB_LDAP_BASE_DN=DC=remotehub,DC=test \
    -e 'REMOTEHUB_ADMIN_GROUPS=RH Admins' \
    -e REMOTEHUB_KRATOS_URL=http://kratos:4433 \
    -e REMOTEHUB_KRATOS_ADMIN_URL=http://kratos:4434 \
    "$1" /ci-target/debug/remotehub
  ci_warten e2e-server 60 bash -c '</dev/tcp/127.0.0.1/8080'
  local rc=0
  docker run --rm --label "ci-lokal=${CI_ID}" --network "container:${CI_ID}-e2e-server" \
    -v "${CI_VOLUME}:${CI_SRC}" -w "${CI_SRC}/web" \
    -e E2E_BASE_URL=http://localhost:8080 -e E2E_SSH_HOST=ssh-target -e CI=1 \
    -e E2E_KRATOS_ADMIN_URL=http://kratos:4434 \
    "$E2E_IMAGE" node node_modules/@playwright/test/cli.js test || rc=$?
  if [ "$rc" != 0 ]; then
    # The run's volume is removed afterwards: keep traces, page snapshots and
    # the server's log next to the CI logs.
    local kept="${CI_PROTOKOLLE}/e2e"
    mkdir -p "$kept"
    docker logs "${CI_ID}-e2e-server" >"${kept}/server.log" 2>&1 || true
    docker run --rm -v "${CI_VOLUME}:${CI_SRC}:ro" "$1" \
      tar -C "${CI_SRC}/web" -cf - test-results | tar -C "$kept" -xf - || true
    echo "Traces and the server log: ${kept} (npx playwright show-trace <trace.zip>)"
  fi
  return "$rc"
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

# Rust (with the lab) and web in parallel, each with its own log; then the
# end-to-end tests with what both built: in full runs, for changes under
# E2E_INPUTS, or with CI_E2E=1. They need the lab and the server binary, so
# without a Rust change the lab starts and the binary is built alone.
# Failed parts are printed last so the summary of gemeinsam.sh shows them.
job_code() {
  local tools logs rust=0 web=0 lab=0 e2e=0 rust_pid="" web_pid="" e2e_pid="" rust_rc=0 web_rc=0 e2e_rc=0
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  logs="$(mktemp -d)"
  if needed "${RUST_INPUTS[@]}"; then rust=1; fi
  if needed "${LAB_INPUTS[@]}"; then lab=1; rust=1; fi
  if needed "${WEB_INPUTS[@]}"; then web=1; fi
  if [ "${CI_E2E:-0}" = 1 ] || needed "${E2E_INPUTS[@]}"; then e2e=1; lab=1; web=1; fi

  if [ "$rust" = 1 ]; then
    ci_dienst db -e POSTGRES_USER=ci -e POSTGRES_PASSWORD=ci -e POSTGRES_DB=ci "$POSTGRES_IMAGE"
    in_background rust part_rust "$tools" "$lab"
  elif [ "$e2e" = 1 ]; then
    in_background rust part_server "$tools"
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

  if [ "$e2e" = 1 ] && [ "$rust_rc" = 0 ] && [ "$web_rc" = 0 ]; then
    in_background e2e part_e2e "$tools"
    wait "$e2e_pid" || e2e_rc=$?
  elif [ "$e2e" = 1 ]; then
    echo "skipped: the end-to-end tests wait for Rust and web to pass" >"${logs}/e2e.log"
  else
    echo "skipped: nothing under ${E2E_INPUTS[*]} changed (CI_E2E=1 forces them)" >"${logs}/e2e.log"
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
