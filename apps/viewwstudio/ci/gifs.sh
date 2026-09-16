#!/usr/bin/env bash
# Record the studio's transitions and encode each to a GIF.
#
# Usage: apps/viewwstudio/ci/gifs.sh out/ [--light] [--fps=25]
#
# The frames come from `examples/motion`, which drives the shell against a
# supplied clock rather than the wall's — so re-running this produces the same
# pictures, and a GIF that differs is a change in the studio and not in the
# machine that recorded it.
set -euo pipefail

out="${1:-motion}"
shift || true

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$root"

cargo run -q -p viewwstudio --example motion -- "$out" "$@"

command -v ffmpeg >/dev/null || { echo "ffmpeg is not installed; frames are in $out" >&2; exit 0; }

for scene in "$out"/*/; do
    name="$(basename "$scene")"
    # Two passes: a palette from the whole sequence, then the encode against
    # it. One pass dithers each frame against its own palette, which on a UI
    # of flat surfaces shows up as the background quietly changing colour.
    ffmpeg -loglevel error -y -framerate 25 -i "$scene/%04d.png" \
        -vf "fps=25,scale=1000:-1:flags=lanczos,palettegen=stats_mode=diff" \
        "$scene/palette.png"
    ffmpeg -loglevel error -y -framerate 25 -i "$scene/%04d.png" -i "$scene/palette.png" \
        -lavfi "fps=25,scale=1000:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=3" \
        -loop 0 "$out/$name.gif"
    echo "$out/$name.gif"
done
