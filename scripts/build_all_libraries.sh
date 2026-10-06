#!/usr/bin/env bash
# Rebuild every asset-library family under assets/library/ (0.9).
#
#   scripts/build_all_libraries.sh            # all glyph + cutout families
#   scripts/build_all_libraries.sh <family>   # one family
#
# Glyph families: assets/glyph_families/<family>.json + the picture font in
# assets/fonts/ -> assets/library/<family>/ (scripts/build_glyph_library.py).
# Cutout families (OpenArt): assets/cutout_families/<family>.json + the work dir
# output/openart/<family>/ (raw/, jobs.tsv) -> assets/library/<family>/
# (scripts/build_family_library.py). Their outputs are committed, so they are
# rebuilt here only when the work dir exists.
# The 0.8 editorial_cutout family is packaged by scripts/build_cutout_library.py
# from its keyed work dir when output/openart/editorial_cutout exists.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q -p motion-cli
export MOTION_ENGINE=target/release/motion-engine
for spec in assets/glyph_families/*.json; do
  fam=$(basename "$spec" .json)
  [ $# -gt 0 ] && [ "$1" != "$fam" ] && continue
  python3 scripts/build_glyph_library.py "$spec" assets/fonts assets
done
for spec in assets/cutout_families/*.json; do
  [ -e "$spec" ] || continue
  fam=$(basename "$spec" .json)
  [ $# -gt 0 ] && [ "$1" != "$fam" ] && continue
  [ -d "output/openart/$fam" ] || continue
  python3 scripts/build_family_library.py "$spec" "output/openart/$fam" assets
done
work=output/openart/editorial_cutout
if [ -d "$work" ] && { [ $# -eq 0 ] || [ "$1" = editorial_cutout ]; }; then
  python3 scripts/build_cutout_library.py "$work" "assets/library/editorial_cutout"
fi
