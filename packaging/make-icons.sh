#!/bin/bash
# Makes every icon file from assets/icon/source.webp: the picture as it was supplied, a
# square on an opaque black ground. Needs ImageMagick and Python 3. Run from the repository
# root after changing the source; the results are committed.
set -euo pipefail

src=assets/icon/source.webp
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Round the corners, leaving them transparent. Cutting along the gold ring instead would
# clip the wings and the pencil, which reach past it.
size=$(identify -format '%w' "$src")
radius=$((size * 18 / 100))
convert "$src" -alpha set \
  \( -size "${size}x${size}" xc:black -fill white \
     -draw "roundrectangle 0,0 $((size - 1)),$((size - 1)) $radius,$radius" \) \
  -alpha off -compose CopyOpacity -composite "$work/master.png"

for n in 16 24 32 48 64 128 256 512; do
  convert "$work/master.png" -filter Lanczos -resize "${n}x${n}" -strip "$work/$n.png"
done

# The window's icon, and the picture in the README.
cp "$work/256.png" assets/icon/icon.png

# The Windows program's icon.
convert "$work/16.png" "$work/24.png" "$work/32.png" "$work/48.png" "$work/64.png" \
  "$work/128.png" "$work/256.png" assets/icon/icon.ico

# The macOS bundle's icon: a list of PNGs, each under the code for its size.
python3 - "$work" packaging/macos/icon.icns <<'PY'
import struct, sys
work, out = sys.argv[1], sys.argv[2]
# Up to 512 pixels. The 1024-pixel entry alone would double the file.
kinds = [("icp4", 16), ("icp5", 32), ("icp6", 64), ("ic07", 128), ("ic08", 256),
         ("ic09", 512), ("ic11", 32), ("ic12", 64), ("ic13", 256)]
body = b""
for code, size in kinds:
    png = open(f"{work}/{size}.png", "rb").read()
    body += code.encode() + struct.pack(">I", len(png) + 8) + png
open(out, "wb").write(b"icns" + struct.pack(">I", len(body) + 8) + body)
PY
