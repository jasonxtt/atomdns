#!/usr/bin/env bash
# Builds the release artifacts for a tag on a Linux x86_64 host: rtdns-linux-<variant>.tar.gz
# (one file, `rtdns`; the variant is amd64 by default), rtdns-webui.tar.gz (the WebUI package:
# the files of <config root>/ui/main/ at the top level) and rtdns-update-manifest.json. Nothing is
# published; attach the files to a GitHub release named like the tag by hand or from CI.
#   scripts/build-rust-release.sh vX.Y.Z [out-dir]
# VARIANTS="amd64 amd64-v3 arm64 armv7" builds more program packages (default: amd64). amd64-v3 is
# the glibc build with -C target-cpu=x86-64-v3; arm64/armv7 are static musl builds made with
# cargo-zigbuild and need qemu-user binfmt to run `rtdns version` (CI: .github/workflows/release-build.yml).
# Only rtdns-linux-amd64.tar.gz is used by the in-app updater; the others are for manual installs.
# PREBUILT=<path to an already built rtdns> replaces the amd64 build (it must report the same tag).
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

for VARIANT in ${VARIANTS:-amd64}; do
  TARGET=""
  FLAGS=""
  SUBCOMMAND="build"
  case "${VARIANT}" in
    amd64) ;;
    amd64-v3) FLAGS="-C target-cpu=x86-64-v3" ;;
    arm64) TARGET="aarch64-unknown-linux-musl"; SUBCOMMAND="zigbuild" ;;
    armv7) TARGET="armv7-unknown-linux-musleabihf"; SUBCOMMAND="zigbuild" ;;
    *) echo "unknown variant ${VARIANT} (amd64, amd64-v3, arm64, armv7)" >&2; exit 1 ;;
  esac
  BIN="${STAGE}/${VARIANT}/rtdns"
  mkdir -p "${STAGE}/${VARIANT}"
  if [ "${VARIANT}" = "amd64" ] && [ -n "${PREBUILT:-}" ]; then
    cp "${PREBUILT}" "${BIN}"
  else
    # Own target dir per variant: the same triple with other RUSTFLAGS must not share build products.
    # An empty CARGO_BUILD_TARGET is not "no target" to cargo: set it only for the cross variants.
    env BUILD_VERSION="${TAG}" OUTPUT="${BIN}" CARGO_TARGET_DIR="${ROOT_DIR}/rust/target/release-${VARIANT}" \
      CARGO_SUBCOMMAND="${SUBCOMMAND}" RUSTFLAGS="${FLAGS}" ${TARGET:+CARGO_BUILD_TARGET="${TARGET}"} \
      "${ROOT_DIR}/scripts/build-rust-native.sh"
  fi
  chmod 0755 "${BIN}"
  # Runs natively, or through qemu-user (binfmt) for the ARM variants.
  REPORTED="$("${BIN}" version)"
  if [ "${REPORTED}" != "${TAG}" ]; then
    echo "the ${VARIANT} binary reports ${REPORTED}, not ${TAG}" >&2
    exit 1
  fi
  ARCHIVE="${OUT}/rtdns-linux-${VARIANT}.tar.gz"
  rm -f "${ARCHIVE}"
  tar -C "${STAGE}/${VARIANT}" --owner=0 --group=0 --numeric-owner -czf "${ARCHIVE}" rtdns
  echo "built ${ARCHIVE}"
done

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
# List once: `tar | grep -q` under pipefail fails when grep exits early and tar gets SIGPIPE.
WEBUI_ENTRIES="$(tar -tzf "${WEBUI}")"
for required in index.html ui-manifest.json; do
  if ! grep -qx "${required}" <<<"${WEBUI_ENTRIES}"; then
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
echo "built ${VARIANTS:-amd64} and ${WEBUI}"
