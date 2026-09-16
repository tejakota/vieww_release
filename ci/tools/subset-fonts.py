from fontTools import subset
import os

SRC = "/usr/share/fonts/truetype/dejavu"
OUT = "crates/vieww-text/assets"
# Latin (incl. punctuation/digits), Latin-1 supplement, Hebrew, Arabic + forms,
# and the whole of General Punctuation.
#
# U+2000-206F replaces the four bidi controls that used to be listed here one at
# a time, and is not a convenience: the block holds the bullet the framework
# itself draws for a masked password field (`DEFAULT_OBSCURING_CHARACTER`, U+2022)
# along with the dashes, quotes and ellipsis any real interface string contains.
# Without it those shape to `.notdef`, and `.notdef` in DejaVu is an *empty*
# glyph with a normal advance — so the text takes its space and draws nothing.
# A password field showed a blank box, which reads as "my typing is not
# registering" rather than as a missing glyph. `the_characters_the_framework_
# draws_are_all_in_the_embedded_font` in `vieww-text` is the guard.
# U+2190-2193 are the four arrows and nothing else in that block: an interface
# string says "← back" often enough to be worth four glyphs, and the rest of
# Arrows is mathematics.
# U+2212 and U+2260-2265 are the seven mathematical characters an *interface*
# actually writes, and none of them were here: a quality gate reading
# "SSIMULACRA2 ≥ 90" rendered as "SSIMULACRA2 90", because DejaVu's `.notdef` is
# an empty glyph with a normal advance — the same failure the General
# Punctuation note above describes, and the same reason it is not visible as a
# missing glyph. The rest of Mathematical Operators is mathematics and stays
# out; these seven are typography.
#   U+2212 minus (the real one, not a hyphen), U+2260 ≠, U+2264 ≤, U+2265 ≥,
#   U+2261 ≡, U+2262 ≢, U+2263 ≣ come free inside the range.
# U+00D7 and U+00F7 (× ÷) are already covered by Latin-1.
# U+2318, U+21E7, U+2325, U+23CE, U+232B are the **Macintosh modifier keys** —
# ⌘ ⇧ ⌥ ⏎ ⌫ — and their absence was the worst-consequence gap in this list.
# `Chord::describe` writes a shortcut as `⌘⇧P` on Apple platforms and `Ctrl+`,
# `Shift+` in words everywhere else, which is correct in both places and only
# ever tested on Linux. On macOS every shortcut in every menu, in the command
# palette, and in the whole Keyboard Shortcuts sheet rendered its modifiers as
# `.notdef` — and DejaVu's `.notdef` is an *empty glyph with a normal advance*,
# the failure mode this file's other notes already describe twice, so the menus
# did not look broken. They looked like a studio whose shortcuts were `P` and
# `S` with a wide space in front of them.
#
# U+25B2-25BE covers the small and large triangles an interface points with:
# the Explorer's own tree chevrons were `▾`/`▸` in the row text and drew tofu
# on every expanded folder. That particular pair is now an icon (see
# `sidebar::Leading`) because a vector icon cannot go missing, but a disclosure
# triangle is ordinary interface typography and the next person to reach for
# one should find it here rather than rediscover this.
#
# U+2713, U+2715 are the check and the cross: a tick beside a chosen item and
# the × on a dismissable chip, both of which the studio writes as text.
#
# U+2039, U+203A are the single angle quotes the find bar uses for previous and
# next.
UNICODES = (
    "U+0020-007E,U+00A0-00FF,U+0590-05FF,U+0600-06FF,U+FB1D-FB4F,U+FE70-FEFF,"
    "U+2000-206F,U+2190-2193,U+2212,U+2260-2265,"
    "U+21E7,U+2318,U+2325,U+23CE,U+232B,U+25B2-25BE,U+2713,U+2715"
)

faces = [
    ("DejaVuSans.ttf", "DejaVuSans-subset.ttf"),
    ("DejaVuSans-Bold.ttf", "DejaVuSans-Bold-subset.ttf"),
    ("DejaVuSans-Oblique.ttf", "DejaVuSans-Oblique-subset.ttf"),
    # A real monospace face, so `FontFamily::Monospace` is a different typeface
    # rather than a different name for the same one. Without it,
    # `db.set_monospace_family(EMBEDDED_FAMILY)` made every monospace request
    # resolve back to DejaVu Sans and the API was a silent no-op — a column of
    # figures still failed to line up, and nothing said why.
    ("DejaVuSansMono.ttf", "DejaVuSansMono-subset.ttf"),
    ("DejaVuSansMono-Bold.ttf", "DejaVuSansMono-Bold-subset.ttf"),
    # **Was in `assets/` and not in this list.** Nothing includes it, so it did
    # not ship in the binary — but it did sit in the repository being a font
    # subset that no run of this script could reproduce, and it silently fell
    # a coverage range behind the other five. Anything in `assets/` that this
    # script does not regenerate is a file whose contents nobody can account
    # for; listing it is cheaper than arguing about deleting it, and it means
    # `assets/` and this list are the same set.
    ("DejaVuSansMono-Oblique.ttf", "DejaVuSansMono-Oblique-subset.ttf"),
]
for src, dst in faces:
    args = [
        os.path.join(SRC, src),
        f"--unicodes={UNICODES}",
        f"--output-file={os.path.join(OUT, dst)}",
        "--layout-features=*",
        "--no-hinting",
        "--desubroutinize",
        "--drop-tables+=DSIG",
        "--name-IDs=*",
    ]
    subset.main(args)
    before = os.path.getsize(os.path.join(SRC, src))
    after = os.path.getsize(os.path.join(OUT, dst))
    print(f"{dst}: {before//1024}KB -> {after//1024}KB")
