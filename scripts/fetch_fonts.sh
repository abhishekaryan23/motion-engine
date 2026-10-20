#!/usr/bin/env bash
# Fetch the optional faces whose licences do not allow them in this repository
# (assets/fonts/RESTRICTED.md). Without them the engine reads open fallbacks
# (assets/fonts/registry.json `restricted_fallbacks`), so this is never required.
#
# usage: scripts/fetch_fonts.sh [--dest <fonts-dir>]     (default: assets/fonts)
#
# Fontshare families come from Fontshare's public download endpoint, one zip per family:
#   https://api.fontshare.com/v2/fonts/download/<slug>
#   slugs: satoshi, cabinet-grotesk, clash-display, panchang, zodiak
# (each answered `200 application/zip` to a HEAD request on 2026-10-21). The fonts are
# licensed under the Fontshare Free Font License (FFL); by running this script you accept it:
# https://www.fontshare.com/licenses
#
# Frantically (1001fonts freeware EULA) has no stable download URL; fetch it by hand from
# https://www.1001fonts.com/frantically-font.html and copy Frantically-Regular.ttf into the
# fonts folder.
#
# Needs curl and unzip. Never run by tests or CI. Existing files are kept.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$ROOT/assets/fonts"
while [ $# -gt 0 ]; do
  case "$1" in
    --dest) DEST=${2:?--dest needs a folder}; shift 2 ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "fetch_fonts: unknown argument $1" >&2; exit 2 ;;
  esac
done
for tool in curl unzip; do
  command -v "$tool" >/dev/null || { echo "fetch_fonts: $tool is required" >&2; exit 2; }
done
mkdir -p "$DEST/licenses"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# slug|licence file name|font files
FAMILIES=(
  "satoshi|Satoshi-FFL.txt|Satoshi-Light.otf Satoshi-Regular.otf Satoshi-Medium.otf Satoshi-Bold.otf"
  "cabinet-grotesk|CabinetGrotesk-FFL.txt|CabinetGrotesk-Light.otf CabinetGrotesk-Regular.otf CabinetGrotesk-Medium.otf CabinetGrotesk-Bold.otf CabinetGrotesk-Extrabold.otf"
  "clash-display|ClashDisplay-FFL.txt|ClashDisplay-Medium.otf ClashDisplay-Semibold.otf ClashDisplay-Bold.otf"
  "panchang|Panchang-FFL.txt|Panchang-Bold.otf Panchang-Extrabold.otf"
  "zodiak|Zodiak-FFL.txt|Zodiak-Thin.otf Zodiak-Light.otf Zodiak-LightItalic.otf Zodiak-Regular.otf Zodiak-Italic.otf Zodiak-Bold.otf"
)

fetched=0
kept=0
for entry in "${FAMILIES[@]}"; do
  IFS='|' read -r slug licence files <<<"$entry"
  need=()
  for f in $files; do
    if [ -f "$DEST/$f" ]; then kept=$((kept + 1)); else need+=("$f"); fi
  done
  [ "${#need[@]}" -eq 0 ] && continue
  url="https://api.fontshare.com/v2/fonts/download/$slug"
  echo "fetch_fonts: $slug <- $url"
  curl -fsSL --retry 2 -o "$TMP/$slug.zip" "$url"
  mkdir -p "$TMP/$slug"
  unzip -q -o "$TMP/$slug.zip" -d "$TMP/$slug"
  for f in "${need[@]}"; do
    src=$(find "$TMP/$slug" -type f -name "$f" | sort | head -n 1)
    if [ -z "$src" ]; then
      echo "fetch_fonts: $f not found in the $slug download" >&2
      exit 1
    fi
    cp "$src" "$DEST/$f"
    fetched=$((fetched + 1))
  done
  if [ ! -f "$DEST/licenses/$licence" ]; then
    lic=$(find "$TMP/$slug" -type f \( -iname '*license*' -o -iname '*licence*' -o -iname 'FFL*' \) | sort | head -n 1)
    [ -n "$lic" ] && cp "$lic" "$DEST/licenses/$licence"
  fi
done

echo "fetch_fonts: $fetched fetched, $kept already present in $DEST"
if [ ! -f "$DEST/Frantically-Regular.ttf" ]; then
  echo "fetch_fonts: Frantically-Regular.ttf is manual: https://www.1001fonts.com/frantically-font.html"
fi
