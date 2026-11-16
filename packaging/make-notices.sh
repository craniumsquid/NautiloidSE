#!/bin/sh
# Writes THIRD-PARTY-LICENSES.txt, the licences of the libraries and fonts built into the
# app, for every download. Run from the repository root:
#
#     sh packaging/make-notices.sh <out file>
#
# Needs cargo-about (cargo install cargo-about --locked --features cli) and jq.
set -eu
out=${1:-THIRD-PARTY-LICENSES.txt}
cargo about generate packaging/third-party.hbs -o "$out"
# egui's default fonts carry two more notices than its declared licences: Hack's and the
# emoji icon font's.
fonts=$(dirname "$(cargo metadata --format-version 1 \
  | jq -r '.packages[] | select(.name == "epaint_default_fonts") | .manifest_path')")/fonts
for notice in Hack-Regular.txt emoji-icon-font-mit-license.txt; do
  printf '\n--------------------------------------------------------------------------------\n' >> "$out"
  printf 'Font notice from epaint_default_fonts (egui): %s\n\n' "$notice" >> "$out"
  cat "$fonts/$notice" >> "$out"
done
