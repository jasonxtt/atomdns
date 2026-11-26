#!/usr/bin/env bash
# Builds the release artifacts for a tag on a Linux x86_64 host: atomdns-linux-<variant>.tar.gz
# (one file, `atomdns`; the variant is amd64 by default), atomdns-webui.tar.gz (the default WebUI
# package: the files of <config root>/ui/main/ at the top level), atomdns-webui-lg.tar.gz (the
# optional glass WebUI package: the files of <config root>/ui/lg/ at the top level) and
# atomdns-update-manifest.json. Nothing is published; attach the files to a GitHub release named
# like the tag by hand or from CI.
#   scripts/build-rust-release.sh vX.Y.Z [out-dir]
# VARIANTS="amd64 amd64-v3 amd64-musl arm64 armv7 arm64-gnu armv7-gnu" builds more program packages (default: amd64).
# amd64-v3 is the glibc build with -C target-cpu=x86-64-v3; amd64-musl (Alpine), arm64 and armv7 are static musl
# builds; arm64-gnu and armv7-gnu are glibc builds (baseline glibc 2.35, used by the Docker image). Every variant uses
# the mimalloc allocator built into the program (feature `mimalloc`), whatever the C library. The ARM variants are made with cargo-zigbuild and need qemu-user binfmt
# (and, for the gnu ones, the ARM glibc sysroot via QEMU_LD_PREFIX) to run `atomdns version`
# (CI: .github/workflows/release-build.yml).
# The in-app updater installs amd64 by default, amd64-v3 on request (or when v3 is running) and amd64-musl
# on musl/Alpine systems; the ARM packages are for manual installs and the Docker image.
# PREBUILT=<path to an already built atomdns> replaces the amd64 build (it must report the same tag).
# PREBUILT_WEBUI=<atomdns-webui.tar.gz> skips the default WebUI build (its ui_version must be the tag).
# PREBUILT_WEBUI_LG=<atomdns-webui-lg.tar.gz> skips the glass WebUI build (its ui_version must be the tag).
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
    amd64-musl) TARGET="x86_64-unknown-linux-musl"; SUBCOMMAND="zigbuild" ;;
    arm64) TARGET="aarch64-unknown-linux-musl"; SUBCOMMAND="zigbuild" ;;
    armv7) TARGET="armv7-unknown-linux-musleabihf"; SUBCOMMAND="zigbuild" ;;
    arm64-gnu) TARGET="aarch64-unknown-linux-gnu.2.35"; SUBCOMMAND="zigbuild" ;;
    armv7-gnu) TARGET="armv7-unknown-linux-gnueabihf.2.35"; SUBCOMMAND="zigbuild" ;;
    *) echo "unknown variant ${VARIANT} (amd64, amd64-v3, amd64-musl, arm64, armv7, arm64-gnu, armv7-gnu)" >&2; exit 1 ;;
  esac
  # qemu-user needs the ARM glibc sysroot to run the gnu programs (also inside build-rust-native.sh's version check)
  case "${VARIANT}" in
    arm64-gnu) export QEMU_LD_PREFIX=/usr/aarch64-linux-gnu ;;
    armv7-gnu) export QEMU_LD_PREFIX=/usr/arm-linux-gnueabihf ;;
    *) unset QEMU_LD_PREFIX ;;
  esac
  BIN="${STAGE}/${VARIANT}/atomdns"
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
  ARCHIVE="${OUT}/atomdns-linux-${VARIANT}.tar.gz"
  rm -f "${ARCHIVE}"
  tar -C "${STAGE}/${VARIANT}" --owner=0 --group=0 --numeric-owner -czf "${ARCHIVE}" atomdns
  echo "built ${ARCHIVE}"
done

check_webui_archive() {
  local archive="$1"
  local label="$2"
  # List once: `tar | grep -q` under pipefail fails when grep exits early and tar gets SIGPIPE.
  local entries
  entries="$(tar -tzf "${archive}")"
  for required in index.html ui-manifest.json; do
    if ! grep -qx "${required}" <<<"${entries}"; then
      echo "${label} has no top-level ${required}" >&2
      exit 1
    fi
  done
  local ui_reported
  ui_reported="$(tar -xzOf "${archive}" ui-manifest.json | jq -r '.ui_version')"
  if [ "${ui_reported}" != "${TAG}" ]; then
    echo "${label} reports ui_version ${ui_reported}, not ${TAG}" >&2
    exit 1
  fi
}

WEBUI="${OUT}/atomdns-webui.tar.gz"
WEBUI_LG="${OUT}/atomdns-webui-lg.tar.gz"
rm -f "${WEBUI}" "${WEBUI_LG}"
if [ -z "${PREBUILT_WEBUI:-}" ] || [ -z "${PREBUILT_WEBUI_LG:-}" ]; then
  (
    cd "${ROOT_DIR}/webui-log"
    npm ci --include=dev
    if [ -z "${PREBUILT_WEBUI:-}" ]; then
      UI_VERSION="${TAG}" npm run build:package
    fi
    if [ -z "${PREBUILT_WEBUI_LG:-}" ]; then
      UI_VERSION="${TAG}" npm run build:lg:package
    fi
  )
fi
if [ -n "${PREBUILT_WEBUI:-}" ]; then
  cp "${PREBUILT_WEBUI}" "${WEBUI}"
else
  cp "${ROOT_DIR}/release/atomdns-webui.tar.gz" "${WEBUI}"
fi
if [ -n "${PREBUILT_WEBUI_LG:-}" ]; then
  cp "${PREBUILT_WEBUI_LG}" "${WEBUI_LG}"
else
  cp "${ROOT_DIR}/release/atomdns-webui-lg.tar.gz" "${WEBUI_LG}"
fi
check_webui_archive "${WEBUI}" "atomdns-webui.tar.gz"
check_webui_archive "${WEBUI_LG}" "atomdns-webui-lg.tar.gz"

"${ROOT_DIR}/scripts/generate-rust-update-manifest.sh" "${TAG}" "${OUT}"
echo "built ${VARIANTS:-amd64}, ${WEBUI} and ${WEBUI_LG}"
