#!/usr/bin/env bash
# Download the pinned PDFium build (bblanchon/pdfium-binaries) into vendor/pdfium/<platform>/.
#
# Pinned to chromium/7881, the release that pdfium-render 0.9.x binds with its `pdfium_latest`
# feature. The archive SHA-256 is verified before extraction.
#
# Usage: scripts/fetch-pdfium.sh [platform] [--force]
#   platform: win-x64 | win-arm64 | mac-arm64 | mac-x64 | linux-x64 | linux-arm64
#             (default: detected from `uname`)
set -euo pipefail

TAG="chromium/7881"
platform=""
force=0
for arg in "$@"; do
  case "$arg" in
    --force) force=1 ;;
    *) platform="$arg" ;;
  esac
done

if [ -z "$platform" ]; then
  os="$(uname -s)"; arch="$(uname -m)"
  case "$os-$arch" in
    Darwin-arm64) platform=mac-arm64 ;;
    Darwin-x86_64) platform=mac-x64 ;;
    Linux-x86_64) platform=linux-x64 ;;
    Linux-aarch64|Linux-arm64) platform=linux-arm64 ;;
    MINGW*-x86_64|MSYS*-x86_64|CYGWIN*-x86_64) platform=win-x64 ;;
    *) echo "Cannot detect platform for $os-$arch; pass one explicitly." >&2; exit 1 ;;
  esac
fi

case "$platform" in
  win-x64)     sha=73cc0de638ac2095e7445bf56a38200a5b7c7ca0e9f4ba144598f2457377ac08; lib=bin/pdfium.dll ;;
  win-arm64)   sha=d3035d4d2cacac6ecd1a2ece197a3d702a1b2a58466276b9f870b8cb278a9d84; lib=bin/pdfium.dll ;;
  mac-arm64)   sha=52e94ca5aa8847934330daf3f8150c190682c5ca93831468794f8b90d4392e40; lib=lib/libpdfium.dylib ;;
  mac-x64)     sha=6dedf83990e0e3d6b7c93c9e7589c5a126b0ae14b7464d76120cff7a26afb18b; lib=lib/libpdfium.dylib ;;
  linux-x64)   sha=1470e21b8b4a3b4ad7f85684e2da11d94f3b69a86d81dee11b9b6709d927ac1d; lib=lib/libpdfium.so ;;
  linux-arm64) sha=ee7f7b7d5468958336a818c1cd580bdd20972846b7377b13f9a923d92d1d4674; lib=lib/libpdfium.so ;;
  *) echo "Unknown platform '$platform'" >&2; exit 1 ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/vendor/pdfium/$platform"
if [ -f "$dest/$lib" ] && [ "$force" -eq 0 ]; then
  echo "PDFium already present: $dest/$lib"
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
url="https://github.com/bblanchon/pdfium-binaries/releases/download/$TAG/pdfium-$platform.tgz"
echo "Downloading $url"
curl --fail --location --silent --show-error -o "$work/pdfium.tgz" "$url"

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$work/pdfium.tgz" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$work/pdfium.tgz" | cut -d' ' -f1)"
fi
if [ "$actual" != "$sha" ]; then
  echo "SHA-256 mismatch for pdfium-$platform.tgz: expected $sha but got $actual" >&2
  exit 1
fi

mkdir -p "$work/extract"
tar -xzf "$work/pdfium.tgz" -C "$work/extract"
rm -rf "$dest"
mkdir -p "$(dirname "$dest")"
mv "$work/extract" "$dest"
echo "bblanchon/pdfium-binaries $TAG ($platform)" > "$dest/VERSION.txt"
[ -f "$dest/$lib" ] || { echo "Expected library not found after extraction: $dest/$lib" >&2; exit 1; }
echo "PDFium $TAG installed: $dest/$lib"
