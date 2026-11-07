#!/usr/bin/env bash
# Collects the assets that scripts/install.sh downloads from a DefaWong/atomdns release.
#
#   ATOMDNS_MUSL_TGZ=release/rust/atomdns-linux-amd64-musl.tar.gz \
#   WEBUI_TGZ=release/rust/atomdns-webui.tar.gz \
#   CONFIG_ZIP=/path/to/config_rust.zip \
#   SINGBOX_SRC=../../golang/sing-box \
#     scripts/package-installer-assets.sh release/installer
#
# Builds the customized sing-box for linux/amd64 (static, CGO disabled) from SINGBOX_SRC,
# copies the other assets and writes SHA256SUMS. scripts/installer-assets.json pins the
# sing-box commit (SINGBOX_SRC must be checked out at it) and the config_rust.zip sha256.
# release-build.yml runs this; publishing a release is a separate, confirmed step
# (docs/rust/installer.md).
set -euo pipefail

OUT="${1:?output directory is required}"
: "${ATOMDNS_MUSL_TGZ:?path to atomdns-linux-amd64-musl.tar.gz}"
: "${WEBUI_TGZ:?path to atomdns-webui.tar.gz}"
: "${CONFIG_ZIP:?path to config_rust.zip}"
: "${SINGBOX_SRC:?path to the customized sing-box checkout}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PINS="$ROOT/scripts/installer-assets.json"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
SINGBOX_SRC="$(cd "$SINGBOX_SRC" && pwd)"

PIN_COMMIT="$(jq -r .singbox.commit "$PINS")"
PIN_VERSION="$(jq -r .singbox.version "$PINS")"
PIN_CONFIG_SHA="$(jq -r .config_full.sha256 "$PINS")"
if command -v sha256sum >/dev/null; then SUM=(sha256sum); else SUM=(shasum -a 256); fi
HEAD="$(git -C "$SINGBOX_SRC" rev-parse HEAD)"
[ "$HEAD" = "$PIN_COMMIT" ] || { echo "$SINGBOX_SRC is at $HEAD, $PINS pins $PIN_COMMIT" >&2; exit 1; }
if [ -n "$(git -C "$SINGBOX_SRC" status --porcelain --untracked-files=no)" ]; then
  echo "$SINGBOX_SRC has uncommitted changes" >&2; exit 1
fi
CONFIG_SHA="$("${SUM[@]}" "$CONFIG_ZIP" | cut -d' ' -f1)"
[ "$CONFIG_SHA" = "$PIN_CONFIG_SHA" ] || { echo "$CONFIG_ZIP sha256 $CONFIG_SHA, $PINS pins $PIN_CONFIG_SHA" >&2; exit 1; }

tar -tzf "$ATOMDNS_MUSL_TGZ" | grep -qx atomdns || { echo "$ATOMDNS_MUSL_TGZ has no top-level atomdns" >&2; exit 1; }
tar -tzf "$WEBUI_TGZ" | grep -qx index.html || { echo "$WEBUI_TGZ has no top-level index.html" >&2; exit 1; }
unzip -l "$CONFIG_ZIP" | grep -q ' config_custom.yaml$' || { echo "$CONFIG_ZIP has no config_custom.yaml" >&2; exit 1; }

TAGS="$(cat "$SINGBOX_SRC/release/DEFAULT_BUILD_TAGS_OTHERS")"
LDFLAGS_SHARED="$(cat "$SINGBOX_SRC/release/LDFLAGS")"
VERSION="${PIN_VERSION}-atomdns.${PIN_COMMIT:0:10}"

BUILD="$(mktemp -d)"
trap 'rm -rf "$BUILD"' EXIT
echo "building sing-box ${VERSION}"
(
  cd "$SINGBOX_SRC"
  GOTOOLCHAIN=local CGO_ENABLED=0 GOOS=linux GOARCH=amd64 GOAMD64=v1 go build -trimpath -tags "$TAGS" \
    -ldflags "-X 'github.com/sagernet/sing-box/constant.Version=${VERSION}' ${LDFLAGS_SHARED} -s -w -buildid=" \
    -o "$BUILD/sing-box" ./cmd/sing-box
)
# macOS bsdtar would add extended-attribute headers that GNU/busybox tar warn about.
TAR_FLAGS=()
if tar --version 2>/dev/null | grep -q bsdtar; then TAR_FLAGS=(--no-xattrs --no-mac-metadata); fi
COPYFILE_DISABLE=1 tar "${TAR_FLAGS[@]}" -czf "$OUT/sing-box-linux-amd64.tar.gz" -C "$BUILD" sing-box

cp "$ATOMDNS_MUSL_TGZ" "$OUT/atomdns-linux-amd64-musl.tar.gz"
cp "$WEBUI_TGZ" "$OUT/atomdns-webui.tar.gz"
cp "$CONFIG_ZIP" "$OUT/config_rust.zip"
cp "$ROOT/scripts/install.sh" "$OUT/install.sh"

(
  cd "$OUT"
  "${SUM[@]}" atomdns-linux-amd64-musl.tar.gz sing-box-linux-amd64.tar.gz atomdns-webui.tar.gz config_rust.zip >SHA256SUMS
  cat SHA256SUMS
)
echo "assets in $OUT"
