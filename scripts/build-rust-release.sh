#!/usr/bin/env bash
# Builds the release artifacts for a tag (linux/amd64 only): rtdns-linux-amd64.tar.gz
# (one file, `rtdns`) and rtdns-update-manifest.json. Nothing is published; attach the
# files to a GitHub release named like the tag by hand or from CI.
#   scripts/build-rust-release.sh vX.Y.Z [out-dir]
# PREBUILT=<path to an already built rtdns> skips the build (it must report the same tag).
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
"${ROOT_DIR}/scripts/generate-rust-update-manifest.sh" "${TAG}" "${OUT}"
echo "built ${ARCHIVE}"
