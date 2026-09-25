#!/usr/bin/env bash
# The files of a release (#142, #158), from its ops package: the package
# under its versioned name and under the name install.sh fetches, install.sh
# from the package, and SHA256SUMS over all three. release.sh publishes them,
# install-check.sh installs from them.
#
#   release-assets.sh OPS_PACKAGE VERSION DIR
set -euo pipefail

ops="$1" version="$2" dir="$3"
rm -rf "$dir"
mkdir -p "$dir"
cp "$ops" "${dir}/remotehub-ops-${version}.tar.gz"
cp "$ops" "${dir}/remotehub-ops.tar.gz"
# From stdin: GNU tar in Git Bash takes "D:/..." for a remote host.
tar -xzOf - remotehub/install.sh <"$ops" >"${dir}/install.sh"
# --text: in Git Bash, sha256sum marks every file binary with "*" otherwise,
# and SHA256SUMS should read the same wherever the release was made.
(cd "$dir" && sha256sum --text "remotehub-ops-${version}.tar.gz" remotehub-ops.tar.gz install.sh >SHA256SUMS)
