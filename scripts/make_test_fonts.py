#!/usr/bin/env python3
"""Build the test fonts for examples/test-text-fidelity and the CJK
embedded-fallback face for crates/vieww-text/assets.

Four jobs:

1. A variable-font subset of Noto Sans SC (wght axis retained) — the
   Phase-2 variable-font demo and its regression test both render the
   same text at two weights through this one face.
2. A static instance of the same subset at wght 400 — the embedded CJK
   fallback face, so the framework's default font set can actually draw
   Chinese headlessly instead of stroking tofu boxes.
3. A CBDT subset of Noto Color Emoji — real colour bitmap strikes for the
   emoji path, small enough to commit.
4. A hand-built COLRv0 layered-colour face — two stacked shapes in two
   palette colours per letter, so the layered path has a font that
   exercises it deterministically.

Run:  python3 scripts/make_test_fonts.py
Writes into examples/test-text-fidelity/assets/ and
crates/vieww-text/assets/.
"""

import math
import os

from fontTools import subset
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
FIDELITY_ASSETS = os.path.join(REPO, "examples", "test-text-fidelity", "assets")
TEXT_ASSETS = os.path.join(REPO, "crates", "vieww-text", "assets")

NOTO_VF = "/usr/share/fonts/truetype/noto-serif-sc/NotoSerifSC-VariableFont_wght.ttf"
NOTO_EMOJI = "/usr/share/fonts/truetype/emoji/NotoColorEmoji.ttf"

# The characters the fidelity suite draws. Latin for the shared baseline,
# enough CJK to be a real shaping exercise, and the punctuation both use.
CJK_TEXT = (
    "视界框架渲染引擎文字排版设计优雅快速现代跨平台中文测试字型回退链表意文字"
    "你好世界字体可变粗细演示画布场景图层阴影渐变模糊混合剪裁图像像素级验证"
)
LATIN_TEXT = "ViewW Variable Weight AaBbGgRr 0123456789 standard typography "
PUNCT = "，。！？、·—…“”（）"

VARIABLE_CHARS = sorted(set(CJK_TEXT + LATIN_TEXT + PUNCT))
EMBEDDED_CHARS = sorted(set(CJK_TEXT + "，。！？、"))


def build_variable_subset():
    """A wght-axis-retaining subset of Noto Sans SC."""
    os.makedirs(FIDELITY_ASSETS, exist_ok=True)
    options = subset.Options()
    options.flavor = []  # keep it a plain TTF
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.notdef_outline = True
    options.recalc_bounds = True
    options.drop_tables += ["DSIG"]
    font = subset.load_font(NOTO_VF, options)
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text="".join(VARIABLE_CHARS))
    subsetter.subset(font)
    out = os.path.join(FIDELITY_ASSETS, "NotoSansSC-VF-subset.ttf")
    font.save(out)
    print(f"wrote {out} ({os.path.getsize(out) // 1024} KB)")
    return out


def build_embedded_cjk():
    """A static wght-400 instance of the CJK subset — the embedded fallback."""
    from fontTools.varLib.instancer import instantiateVariableFont

    os.makedirs(TEXT_ASSETS, exist_ok=True)
    options = subset.Options()
    options.flavor = []
    options.layout_features = ["*"]
    options.name_IDs = [1, 2, 4, 6]
    options.notdef_outline = True
    font = subset.load_font(NOTO_VF, options)
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text="".join(EMBEDDED_CHARS))
    subsetter.subset(font)
    # Pin every axis: the embedded set is a *static* face at the reading
    # weight, so it behaves exactly like the DejaVu faces beside it.
    axes = {axis.axisTag: axis.defaultValue for axis in font["fvar"].axes}
    instantiateVariableFont(font, axes, inplace=True)
    out = os.path.join(TEXT_ASSETS, "NotoSansCJK-subset.ttf")
    font.save(out)
    print(f"wrote {out} ({os.path.getsize(out) // 1024} KB)")
    return out


def build_emoji_subset():
    """A CBDT subset of Noto Color Emoji for a handful of emoji."""
    os.makedirs(FIDELITY_ASSETS, exist_ok=True)
    options = subset.Options()
    # Bitmap tables (CBDT/CBLC) are subsetted by default.
    options.drop_tables += ["SVG ", "GSUB", "GPOS", "DSIG"]
    # Keep the PostScript name (id 6): fontdb refuses a face without one,
    # silently — `parse_names` answers `None` and the load is dropped. This
    # cost a debugging round-trip that the comment now saves for the next
    # person.
    options.name_IDs = [1, 2, 4, 6]
    font = subset.load_font(NOTO_EMOJI, options)
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text="🚀🎉❤️😀🔥✨👍🌈⚡🎯💡🏆🖼")
    subsetter.subset(font)
    out = os.path.join(FIDELITY_ASSETS, "NotoColorEmoji-subset.ttf")
    font.save(out)
    print(f"wrote {out} ({os.path.getsize(out) // 1024} KB)")
    return out


def build_colr_v0():
    """A hand-built COLRv0 face: each 'letter' is two stacked coloured shapes.

    Glyphs: 'A' (disc over square), 'B' (square over disc), 'C' (disc over
    offset square). Layer shapes live in the same glyf table, referenced by
    the COLR records; cmap maps A/B/C to their base glyph ids.
    """
    os.makedirs(FIDELITY_ASSETS, exist_ok=True)
    upem = 1000
    fb = FontBuilder(upem, isTTF=True)
    fb.setupGlyphOrder([".notdef", "disc", "square", "A", "B", "C"])
    fb.setupCharacterMap({0x41: "A", 0x42: "B", 0x43: "C"})

    # disc: a 32-gon circle, r=300, centred at (300, 420).
    pen = TTGlyphPen(None)
    pen.moveTo((600, 420))
    for step in range(1, 32):
        angle = step * (2 * math.pi / 32)
        pen.lineTo((300 + 300 * math.cos(angle), 420 + 300 * math.sin(angle)))
    pen.closePath()
    disc = pen.glyph()

    # square: 520x520 at (40, 120).
    pen = TTGlyphPen(None)
    pen.moveTo((40, 120))
    pen.lineTo((560, 120))
    pen.lineTo((560, 640))
    pen.lineTo((40, 640))
    pen.closePath()
    square = pen.glyph()

    # COLR base glyphs conventionally carry empty outlines — every visible
    # pixel comes from the layer records. A/B/C therefore are blank advance
    # carriers, and the layers below are where the shapes live.
    empty = TTGlyphPen(None).glyph()

    fb.setupGlyf(
        {
            ".notdef": TTGlyphPen(None).glyph(),
            "disc": disc,
            "square": square,
            "A": empty,
            "B": empty,
            "C": empty,
        }
    )
    fb.setupHorizontalMetrics(
        {name: (600, 40) for name in [".notdef", "disc", "square", "A", "B", "C"]}
    )
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    # Every id fontdb and fontTools' subsetter expect: family (1), style (2),
    # full name (4), unique id (3), version (5) and — the one whose absence
    # makes fontdb silently refuse the whole face — PostScript name (6).
    fb.setupNameTable(
        {
            "familyName": "ViewwColourTest",
            "styleName": "Regular",
            "fullName": "ViewwColourTest Regular",
            "uniqueFontIdentifier": "ViewwColourTest Regular 1.0",
            "version": "Version 1.0",
            "psName": "ViewwColourTest-Regular",
        },
    )
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)

    # `fontTools.colorLib.builder` is the supported way to raise a plain
    # mapping into a compilable COLR/CPAL pair. The v0 shape of the input —
    # base glyph -> [(layer glyph, palette index), ...] — compiles to a
    # version-0 table.
    from fontTools.colorLib.builder import buildCOLR, buildCPAL

    fb.font["COLR"] = buildCOLR(
        {
            "A": [("square", 0), ("disc", 1)],
            "B": [("disc", 2), ("square", 3)],
            "C": [("square", 4), ("disc", 5)],
        }
    )
    # `buildCPAL` takes float colours in [0..1]; the on-disk table is
    # 8-bit BGRA either way.
    to_float = lambda rgba: tuple(c / 255.0 for c in rgba)
    fb.font["CPAL"] = buildCPAL(
        [
            [
                to_float((255, 0, 255, 255)),  # 0 magenta
                to_float((0, 200, 255, 255)),  # 1 cyan
                to_float((255, 180, 0, 255)),  # 2 amber
                to_float((150, 60, 255, 255)),  # 3 violet
                to_float((40, 200, 60, 255)),  # 4 green
                to_float((255, 50, 60, 255)),  # 5 red
            ]
        ]
    )

    out = os.path.join(FIDELITY_ASSETS, "ViewwColourTest-COLRv0.ttf")
    fb.save(out)
    print(f"wrote {out} ({os.path.getsize(out)} B)")
    return out


if __name__ == "__main__":
    build_variable_subset()
    build_embedded_cjk()
    build_emoji_subset()
    build_colr_v0()
    print("all fonts built")
