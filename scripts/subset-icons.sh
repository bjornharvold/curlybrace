#!/usr/bin/env bash
# Regenerates crates/ui/assets/fonts/MaterialSymbolsSubset.ttf from the upstream Material Symbols
# Outlined variable font (Apache 2.0, google/material-design-icons).
#
# Pipeline: pin the variable axes to a static instance (fontTools.varLib.instancer), then keep only
# the glyphs listed in GLYPHS (pyftsubset). The glyph list MUST match the constants in
# crates/ui/src/theme/icons.rs.
#
# Requirements: python3 and curl. The script creates a throwaway virtualenv in a temp directory,
# installs fonttools + brotli there, and removes it on exit. Nothing is installed into the repo or
# the system. Set SOURCE_FONT=/path/to/MaterialSymbolsOutlined[...].ttf to skip the download.
#
# Instance: FILL=0 (outlined), wght=400, GRAD=0, opsz=24.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/crates/ui/assets/fonts/MaterialSymbolsSubset.ttf"
UPSTREAM='https://github.com/google/material-design-icons/raw/master/variablefont/MaterialSymbolsOutlined%5BFILL,GRAD,opsz,wght%5D.ttf'

# name:codepoint (Material Symbols codepoints)
GLYPHS=(
  settings:e8b8          # GEAR
  check:e5ca             # CHECK
  chevron_right:e5cc     # CHEVRON_RIGHT
  expand_more:e5cf       # CHEVRON_DOWN
  close:e5cd             # CLOSE
  warning:e002           # WARNING
  error:e000             # ERROR
  info:e88e              # INFO
  arrow_upward:e5d8      # ARROW_UP (sort ascending)
  arrow_downward:e5db    # ARROW_DOWN (sort descending)
)

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

python3 -m venv "$WORK/venv"
"$WORK/venv/bin/pip" install --quiet fonttools brotli

SRC="${SOURCE_FONT:-$WORK/MaterialSymbolsOutlined.ttf}"
if [[ -z "${SOURCE_FONT:-}" ]]; then
  curl -fsSL -o "$SRC" "$UPSTREAM"
fi

UNICODES=""
for g in "${GLYPHS[@]}"; do
  cp="${g#*:}"
  UNICODES+="U+${cp%% *},"
done
UNICODES="${UNICODES%,}"

"$WORK/venv/bin/fonttools" varLib.instancer "$SRC" FILL=0 GRAD=0 opsz=24 wght=400 \
  -o "$WORK/instance.ttf"

# No layout features / names: icons are addressed by codepoint only.
"$WORK/venv/bin/pyftsubset" "$WORK/instance.ttf" \
  --unicodes="$UNICODES" \
  --layout-features= \
  --no-hinting \
  --desubroutinize \
  --name-IDs= \
  --notdef-outline \
  --output-file="$OUT"

echo "wrote $OUT ($(wc -c <"$OUT") bytes)"
