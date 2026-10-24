#!/usr/bin/env bash
# Builds the release artifacts for a tag (linux/amd64 only): rtdns-linux-amd64.tar.gz
# (one file, `rtdns`), rtdns-webui.tar.gz (the WebUI package: the files of
# <config root>/ui/main/ at the top level) and rtdns-update-manifest.json. Nothing is
# published; attach the files to a GitHub release named like the tag by hand or from CI.
#   scripts/build-rust-release.sh vX.Y.Z [out-dir]
# PREBUILT=<path to an already built rtdns> skips the build (it must report the same tag).
# PREBUILT_WEBUI=<rtdns-webui.tar.gz> skips the WebUI build (its ui_version must be the tag).
# RUST_CONFIG_PACKAGE=<config_rust_up.zip> is passed on to generate-rust-update-manifest.sh.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${1:?release tag is required (vX.Y.Z)}"
OUT="${2:-${ROOT_DIR}/release/rust}"

if ! [[ "${TAG}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "tag must look like vX.Y.Z, got ${TAG}" >&2
  exit 1
fi
if [ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]; then
  echo "Release artifacts are built for linux/amd64 only" >&2
  exit 1
fi

mkdir -p "${OUT}"
OUT="$(cd "${OUT}" && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

if [ -n "${PREBUILT:-}" ]; then
  cp "${PREBUILT}" "${STAGE}/rtdns"
else
  BUILD_VERSION="${TAG}" OUTPUT="${STAGE}/rtdns" "${ROOT_DIR}/scripts/build-rust-native.sh"
fi
chmod 0755 "${STAGE}/rtdns"
REPORTED="$("${STAGE}/rtdns" version)"
if [ "${REPORTED}" != "${TAG}" ]; then
  echo "the binary reports ${REPORTED}, not ${TAG}" >&2
  exit 1
fi

ARCHIVE="${OUT}/rtdns-linux-amd64.tar.gz"
rm -f "${ARCHIVE}"
tar -C "${STAGE}" --owner=0 --group=0 --numeric-owner -czf "${ARCHIVE}" rtdns

WEBUI="${OUT}/rtdns-webui.tar.gz"
rm -f "${WEBUI}"
if [ -n "${PREBUILT_WEBUI:-}" ]; then
  cp "${PREBUILT_WEBUI}" "${WEBUI}"
else
  (
    cd "${ROOT_DIR}/webui-log"
    npm ci --include=dev
    UI_VERSION="${TAG}" npm run build:package
  )
  cp "${ROOT_DIR}/release/rtdns-webui.tar.gz" "${WEBUI}"
fi
for required in index.html ui-manifest.json; do
  if ! tar -tzf "${WEBUI}" | grep -qx "${required}"; then
    echo "rtdns-webui.tar.gz has no top-level ${required}" >&2
    exit 1
  fi
done
UI_REPORTED="$(tar -xzOf "${WEBUI}" ui-manifest.json | jq -r '.ui_version')"
if [ "${UI_REPORTED}" != "${TAG}" ]; then
  echo "rtdns-webui.tar.gz reports ui_version ${UI_REPORTED}, not ${TAG}" >&2
  exit 1
fi

"${ROOT_DIR}/scripts/generate-rust-update-manifest.sh" "${TAG}" "${OUT}"
echo "built ${ARCHIVE} and ${WEBUI}"
