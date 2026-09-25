#!/usr/bin/env bash
# Releases remotehub from the head of main, locally like the CI:
#
#   bash scripts/ci/release.sh 0.1.0             # check, build, try, publish
#   bash scripts/ci/release.sh 0.1.0 --dry-run   # check, build and try only
#
# 1. The head of origin/main is checked out and clean, Cargo.toml and
#    deploy/ops/.env.example name the version, and vVERSION is new.
# 2. The commit has the status `lokal`: success (bash scripts/ci/lokal.sh).
# 3. The images are built from the commit and tried with the ops package
#    in a fresh compose project (scripts/ci/image-check.sh).
# 4. The images go to GHCR, and a GitHub release vVERSION gets generated
#    notes and the ops package as remotehub-ops-VERSION.tar.gz.
#
# Needs git, docker, gh (signed in) and `docker login ghcr.io` with a token
# that may write packages. The version is bumped in its own pull request
# beforehand.

CI_REPO_KURZ="remotehub"
# shellcheck source=scripts/ci/gemeinsam.sh
source "$(dirname "${BASH_SOURCE[0]}")/gemeinsam.sh"

REGISTRY="ghcr.io/hilman2"
IMAGE="${REGISTRY}/remotehub"
GUACD_IMAGE="${REGISTRY}/remotehub-guacd"
BROWSER_IMAGE="${REGISTRY}/remotehub-browser"

version="${1:-}"
dry_run=0
[ "${2:-}" = "--dry-run" ] && dry_run=1
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  ci_fehler "Usage: bash scripts/ci/release.sh X.Y.Z [--dry-run]"
tag="v${version}"

# In a dry run, a failed precondition is reported and the run goes on.
precondition() { # message
  if [ "$dry_run" = 1 ]; then
    echo "! $* (ignored in a dry run)"
  else
    ci_fehler "$*"
  fi
}

echo "── Preconditions for ${tag}"
CI_WURZEL="$(git rev-parse --show-toplevel)"
cd "$CI_WURZEL"
git fetch -q origin main --tags
CI_SHA="$(git rev-parse HEAD)"
CI_SHA_KURZ="${CI_SHA:0:7}"
CI_ABLAGE="$(git rev-parse --path-format=absolute --git-common-dir)/ci-lokal"
mkdir -p "$CI_ABLAGE"
CI_GITHUB="$(gh repo view --json nameWithOwner -q .nameWithOwner)"

[ "$CI_SHA" = "$(git rev-parse origin/main)" ] ||
  precondition "HEAD is not the head of origin/main"
[ -z "$(git status --porcelain)" ] || precondition "the working copy is not clean"
cargo_version="$(git show "${CI_SHA}:Cargo.toml" | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1)"
[ "$cargo_version" = "$version" ] || ci_fehler "Cargo.toml has version ${cargo_version}, not ${version}"
ops_version="$(git show "${CI_SHA}:deploy/ops/.env.example" | sed -n 's/^REMOTEHUB_VERSION=//p')"
[ "$ops_version" = "$version" ] ||
  ci_fehler "deploy/ops/.env.example has REMOTEHUB_VERSION=${ops_version}, not ${version}"
if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null || gh release view "$tag" >/dev/null 2>&1; then
  ci_fehler "${tag} exists already"
fi
status="$(gh api "repos/${CI_GITHUB}/commits/${CI_SHA}/status" \
  --jq ".statuses[] | select(.context == \"${CI_KONTEXT}\") | .state" | head -n 1)"
[ "$status" = success ] ||
  precondition "${CI_SHA_KURZ} has no green status \"${CI_KONTEXT}\" (${status:-none}): bash scripts/ci/lokal.sh"
echo "${CI_SHA_KURZ} $(git log -1 --format=%s "$CI_SHA")"

trap ci_aufraeumen EXIT
trap 'exit 130' INT TERM
ci_sperren
ci_umgebung

echo "── Build and try the images"
tools="$(ci_image scripts/ci/tools.Dockerfile)"
# --pull: the base images as they are today, not as the build cache has them.
ci_docker_run "$tools" bash scripts/ci/image-check.sh "$version" "$IMAGE" "$GUACD_IMAGE" "$BROWSER_IMAGE" \
  "$version" --pull

ops="${CI_ABLAGE}/remotehub-ops-${version}.tar.gz"
git archive --format=tar.gz --prefix=remotehub/ -o "$ops" "${CI_SHA}:deploy/ops"
echo "ops package: ${ops}"
# install.sh (#142) fetches the package under a name without the version,
# from the latest release or a given one, and checks it against SHA256SUMS.
assets="${CI_ABLAGE}/assets-${version}"
rm -rf "$assets"
mkdir -p "$assets"
cp "$ops" "${assets}/remotehub-ops-${version}.tar.gz"
cp "$ops" "${assets}/remotehub-ops.tar.gz"
git show "${CI_SHA}:deploy/ops/install.sh" >"${assets}/install.sh"
(cd "$assets" && sha256sum remotehub-ops-"${version}".tar.gz remotehub-ops.tar.gz install.sh >SHA256SUMS)

if [ "$dry_run" = 1 ]; then
  echo "✓ dry run: ${IMAGE}:${version}, ${GUACD_IMAGE}:${version} and ${BROWSER_IMAGE}:${version} built and tried, nothing published"
  exit 0
fi

echo "── Publish"
docker push -q "${IMAGE}:${version}"
docker push -q "${GUACD_IMAGE}:${version}"
docker push -q "${BROWSER_IMAGE}:${version}"
notes="Install on a Debian or Ubuntu host: \`curl -fsSL https://github.com/${CI_GITHUB}/releases/download/${tag}/install.sh | sudo sh -s -- --version ${version}\`
Images: \`${IMAGE}:${version}\`, \`${GUACD_IMAGE}:${version}\`, \`${BROWSER_IMAGE}:${version}\`.
Install: [docs/install.md](https://github.com/${CI_GITHUB}/blob/${tag}/docs/install.md), upgrade: [docs/install.md#upgrade](https://github.com/${CI_GITHUB}/blob/${tag}/docs/install.md#upgrade)."
gh release create "$tag" "${assets}"/* --target "$CI_SHA" --title "remotehub ${version}" \
  --notes "$notes" --generate-notes

# GHCR makes a new package private; installations pull without signing in.
# An anonymous pull of the manifest shows whether they can.
for repository in remotehub remotehub-guacd remotehub-browser; do
  token="$(curl -fsS "https://ghcr.io/token?scope=repository:hilman2/${repository}:pull" 2>/dev/null |
    sed -n 's/.*"token":"\([^"]*\)".*/\1/p' || true)"
  if ! curl -fsS -o /dev/null -H "Authorization: Bearer ${token}" \
    -H "Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json" \
    "https://ghcr.io/v2/hilman2/${repository}/manifests/${version}" 2>/dev/null; then
    echo "! ${REGISTRY}/${repository} is not public yet: https://github.com/users/hilman2/packages/container/${repository}/settings"
  fi
done
echo "✓ released ${tag}: https://github.com/${CI_GITHUB}/releases/tag/${tag}"
