#!/usr/bin/env bash
# Builds (and optionally pushes) the AtomDNS container image from the files of a published GitHub release.
# Nothing is compiled: the static musl programs, the WebUI package, the fresh-install configuration and the
# incremental configuration package are downloaded, checked against the release manifest, staged next to
# docker/Dockerfile and wrapped by `docker buildx`.
#   scripts/build-docker-image.sh vX.Y.Z
# Environment:
#   IMAGE=jasonxtt/atomdns          repository (tags: vX.Y.Z, plus latest when LATEST=1)
#   PLATFORMS=linux/amd64,linux/arm64,linux/arm/v7
#   PUSH=0                          1 pushes the multi-platform image; 0 builds the first platform into the
#                                   local docker image store (tag <IMAGE>:<tag>) so it can be run and checked
#   LATEST=0                        1 also tags :latest
#   RELEASE_DIR=<dir>               use already downloaded release files instead of `gh release download`
#   REPO=jasonxtt/atomdns
# Pushing is publishing: run it only after the release has been checked and the push approved.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${1:?release tag is required (vX.Y.Z)}"
if ! [[ "${TAG}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "tag must look like vX.Y.Z, got ${TAG}" >&2
  exit 1
fi
IMAGE="${IMAGE:-jasonxtt/atomdns}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64,linux/arm/v7}"
PUSH="${PUSH:-0}"
LATEST="${LATEST:-0}"
REPO="${REPO:-jasonxtt/atomdns}"

STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT
SRC="${RELEASE_DIR:-${STAGE}/release}"
mkdir -p "${SRC}" "${STAGE}/ctx/bin" "${STAGE}/ctx/assets"

# platform -> staging directory name, program package
PACKAGES=()
for platform in ${PLATFORMS//,/ }; do
  case "${platform}" in
    linux/amd64) PACKAGES+=("amd64:atomdns-linux-amd64-musl.tar.gz") ;;
    linux/arm64) PACKAGES+=("arm64:atomdns-linux-arm64.tar.gz") ;;
    linux/arm/v7) PACKAGES+=("armv7:atomdns-linux-armv7.tar.gz") ;;
    *) echo "unsupported platform ${platform} (linux/amd64, linux/arm64, linux/arm/v7)" >&2; exit 1 ;;
  esac
done

if [ -z "${RELEASE_DIR:-}" ]; then
  patterns=(--pattern atomdns-webui.tar.gz --pattern atomdns-update-manifest.json --pattern config_rust.zip)
  for pair in "${PACKAGES[@]}"; do patterns+=(--pattern "${pair#*:}"); done
  gh release download "${TAG}" --repo "${REPO}" --dir "${SRC}" "${patterns[@]}"
fi

MANIFEST="${SRC}/atomdns-update-manifest.json"
[ "$(jq -r '.version' "${MANIFEST}")" = "${TAG}" ] || { echo "the manifest is not for ${TAG}" >&2; exit 1; }

sha256_of() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
check_asset() { # <file> <name in the manifest>
  local want
  want="$(jq -r --arg n "$2" '.artifacts[$n].sha256 // empty' "${MANIFEST}")"
  [ -n "${want}" ] || { echo "the manifest has no sha256 for $2" >&2; exit 1; }
  [ "$(sha256_of "$1")" = "${want}" ] || { echo "sha256 mismatch for $2" >&2; exit 1; }
}

# program packages -> bin/<arch>/atomdns
for pair in "${PACKAGES[@]}"; do
  arch="${pair%%:*}"; name="${pair#*:}"
  check_asset "${SRC}/${name}" "${name}"
  mkdir -p "${STAGE}/ctx/bin/${arch}"
  tar -xzf "${SRC}/${name}" -C "${STAGE}/ctx/bin/${arch}" atomdns
done

# image assets
check_asset "${SRC}/atomdns-webui.tar.gz" atomdns-webui.tar.gz
cp "${SRC}/atomdns-webui.tar.gz" "${STAGE}/ctx/assets/"
# The fresh-install package is a release asset but not part of the manifest: it must carry the same WebUI.
cp "${SRC}/config_rust.zip" "${STAGE}/ctx/assets/"
UI_IN_ZIP="$(unzip -p "${SRC}/config_rust.zip" ui/main/ui-manifest.json | jq -r '.ui_version')"
[ "${UI_IN_ZIP}" = "${TAG}" ] || { echo "config_rust.zip carries WebUI ${UI_IN_ZIP}, not ${TAG}" >&2; exit 1; }
# The incremental package is pinned by url + sha256 in the manifest.
CONFIG_URL="$(jq -r '.config.url // empty' "${MANIFEST}")"
CONFIG_SHA="$(jq -r '.config.sha256 // empty' "${MANIFEST}")"
[ -n "${CONFIG_URL}" ] && [ -n "${CONFIG_SHA}" ] || { echo "the manifest has no configuration package" >&2; exit 1; }
curl -fsSL --retry 3 -o "${STAGE}/ctx/assets/config_rust_up.zip" "${CONFIG_URL}"
[ "$(sha256_of "${STAGE}/ctx/assets/config_rust_up.zip")" = "${CONFIG_SHA}" ] || { echo "sha256 mismatch for config_rust_up.zip" >&2; exit 1; }

cp "${ROOT_DIR}/docker/Dockerfile" "${STAGE}/ctx/Dockerfile"
TAGS=(-t "${IMAGE}:${TAG}")
[ "${LATEST}" = "1" ] && TAGS+=(-t "${IMAGE}:latest")
VCS_REF="$(git -C "${ROOT_DIR}" rev-parse --short=7 HEAD 2>/dev/null || echo unknown)"

if [ "${PUSH}" = "1" ]; then
  docker buildx build --platform "${PLATFORMS}" --push "${TAGS[@]}" \
    --build-arg "VERSION=${TAG}" --build-arg "VCS_REF=${VCS_REF}" "${STAGE}/ctx"
  docker buildx imagetools inspect "${IMAGE}:${TAG}"
else
  FIRST="${PLATFORMS%%,*}"
  docker buildx build --platform "${FIRST}" --load "${TAGS[@]}" \
    --build-arg "VERSION=${TAG}" --build-arg "VCS_REF=${VCS_REF}" "${STAGE}/ctx"
  echo "built ${IMAGE}:${TAG} for ${FIRST} (local only; PUSH=1 publishes)"
fi
