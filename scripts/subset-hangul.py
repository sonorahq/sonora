#!/usr/bin/env python3
"""Cut assets/fonts/SonoraHangul.ttf out of Pretendard's variable font.

Inter already draws Latin, so the app only needs Pretendard's Hangul, and only over the
weights the interface uses. Pretendard reserves its name under the OFL, so the cut font is
renamed. Pass the path to PretendardVariable.ttf from the Pretendard 1.3.9 release at
https://github.com/orioncactus/pretendard/releases and run it with fontTools installed:

    nix shell --impure --expr 'with import <nixpkgs> {}; python3.withPackages (p: [p.fonttools])' \
        --command python3 scripts/subset-hangul.py PretendardVariable.ttf
"""

import io
import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "assets" / "fonts" / "SonoraHangul.ttf"

FAMILY = "Sonora Hangul"
POSTSCRIPT = "SonoraHangul"

# Regular through Bold, the weights Inter is embedded in.
WEIGHTS = instancer.AxisTriple(400, 400, 700)

# Hangul Jamo, Compatibility Jamo, Jamo Extended-A, Syllables, Jamo Extended-B and the
# halfwidth Hangul letters.
HANGUL = [
    *range(0x1100, 0x1200),
    *range(0x3130, 0x3190),
    *range(0xA960, 0xA980),
    *range(0xAC00, 0xD7A4),
    *range(0xD7B0, 0xD800),
    *range(0xFFA0, 0xFFDD),
]

# Copyright, trademark, license and license URL keep Pretendard's own wording.
UNTOUCHED_NAMES = {0, 7, 13, 14}


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: subset-hangul.py PretendardVariable.ttf")

    font = TTFont(sys.argv[1])
    font = reload(instancer.instantiateVariableFont(font, {"wght": WEIGHTS}))

    options = subset.Options()
    options.layout_features = ["*"]
    options.hinting = False
    options.name_IDs = ["*"]
    options.name_languages = ["*"]
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=HANGUL)
    subsetter.subset(font)

    rename(font)
    font.save(OUT)
    print(f"{OUT.relative_to(ROOT)}: {OUT.stat().st_size} bytes")


def reload(font: TTFont) -> TTFont:
    """Round-trips a font through bytes, since the subsetter trips over the lazy tables the
    instancer leaves behind."""
    buffer = io.BytesIO()
    font.save(buffer)
    buffer.seek(0)
    return TTFont(buffer)


def rename(font: TTFont) -> None:
    """Swaps every Pretendard name for the reserved-name-free one the OFL requires."""
    for record in font["name"].names:
        if record.nameID in UNTOUCHED_NAMES:
            continue
        text = record.toUnicode()
        renamed = (
            text.replace("PretendardVariable", POSTSCRIPT)
            .replace("Pretendard Variable", FAMILY)
            .replace("Pretendard", FAMILY)
        )
        if renamed != text:
            record.string = renamed


if __name__ == "__main__":
    main()
