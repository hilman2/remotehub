#!/usr/bin/env bash
# Local CI for remotehub — the repository's only CI (no Actions workflows).
# Usage and options: bash scripts/ci/lokal.sh --help
#
# gemeinsam.sh is the shared scaffold, kept verbatim across all of the
# maintainer's repositories (German on purpose); only this file is specific
# to remotehub.

CI_REPO_KURZ="remotehub"
CI_JOBS=(base)

# shellcheck source=scripts/ci/gemeinsam.sh
source "$(dirname "${BASH_SOURCE[0]}")/gemeinsam.sh"

# CI scripts and line endings.
job_base() {
  local tools
  tools="$(ci_image scripts/ci/tools.Dockerfile)"
  ci_docker_run "$tools" bash -euo pipefail -c '
    echo "── ShellCheck"
    shellcheck --version | head -2
    shellcheck -x -P SCRIPTDIR -S warning scripts/ci/*.sh

    echo "── Line endings"
    if grep -rl $'"'"'\r'"'"' scripts/ci; then
      echo "CRLF in scripts/ci"
      exit 1
    fi
  '
}

ci_main "$@"
