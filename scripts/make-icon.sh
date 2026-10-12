#!/usr/bin/env bash
# Regenerate every icon asset from the master SVG (assets/icon/app-icon.svg).
#
#   scripts/make-icon.sh
#
# Needs macOS (iconutil) and rsvg-convert (`brew install librsvg`). The generated files are
# committed, so contributors and CI never need to run this unless the master SVG changes.
#
# Writes:
#   assets/icon/AppIcon.icns          macOS app icon, bundled by scripts/package-macos.sh
#   assets/icon/app-icon-256.png      README image and the window icon embedded in the binary
#   site/favicon.svg                  browser tab icon (body only, no shadow)
#   site/apple-touch-icon.png         180x180 full-bleed square for iOS home screens
set -euo pipefail

cd "$(dirname "$0")/.."
SRC="assets/icon/app-icon.svg"
OUT="assets/icon"
command -v rsvg-convert >/dev/null || { echo "rsvg-convert not found (brew install librsvg)" >&2; exit 1; }
command -v iconutil >/dev/null || { echo "iconutil not found (this script needs macOS)" >&2; exit 1; }

render() { rsvg-convert -w "$1" -h "$1" "$2" -o "$3"; }

# --- .icns ---------------------------------------------------------------------------------
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  render "$size" "$SRC" "$ICONSET/icon_${size}x${size}.png"
  render "$((size * 2))" "$SRC" "$ICONSET/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$OUT/AppIcon.icns"

# --- PNGs ----------------------------------------------------------------------------------
render 256 "$SRC" "$OUT/app-icon-256.png"

# --- site assets: crop to the icon body (drop the 100px margin and the drop shadow) ----------
TMP_SVG="$(mktemp).svg"
sed -e 's/viewBox="0 0 1024 1024"/viewBox="100 100 824 824"/' \
    -e 's/ filter="url(#shadow)"//' "$SRC" > "$TMP_SVG"
cp "$TMP_SVG" site/favicon.svg

# iOS rounds the corners itself, so give it a full-bleed square.
sed -e 's/rx="188"/rx="0"/g' -e 's/rx="187"/rx="0"/g' "$TMP_SVG" > "$TMP_SVG.square.svg"
render 180 "$TMP_SVG.square.svg" site/apple-touch-icon.png

echo "icons regenerated:"
ls -la "$OUT" site/favicon.svg site/apple-touch-icon.png
