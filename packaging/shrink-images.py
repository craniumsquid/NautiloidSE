#!/usr/bin/env python3
"""Shrinks the guide's pictures to 256 colours, which the app's flat colours take well, so
they cost a fifth of the space. Run from the repository root after the guide test has
written them:

    NAUTILOID_GUIDE_IMAGES=1 cargo test --test guide
    python3 packaging/shrink-images.py docs/images/guide/*.png

Needs Pillow (pip install pillow).
"""

import sys

from PIL import Image

for path in sys.argv[1:]:
    picture = Image.open(path).convert("RGB")
    picture.quantize(
        colors=256, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE
    ).save(path, optimize=True)
    print(path)
