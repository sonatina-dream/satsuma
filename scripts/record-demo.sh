#!/bin/sh
# Regenerates the README media: docs/demo.mp4, docs/demo.gif, docs/screenshots/*.png and
# docs/icon.png.
#
# Nothing is recorded from the screen. `satsuma render-demo` (built with the `demo` feature)
# draws a plain desktop with invented sample files, renders the app's own wheel, progress and
# editor views into it offscreen and moves a scripted pointer. Samples and the lossless master
# video live in a throw-away folder that is deleted afterwards. Needs ffmpeg.
set -eu

cd "$(dirname "$0")/.."
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cargo build --release -p satsuma-app --features demo
BIN=target/release/satsuma

mkdir -p docs/screenshots
"$BIN" render-demo "$WORK" docs
"$BIN" render-icon --size 256 --out docs/icon.png

ffmpeg -y -loglevel error -i "$WORK/master.mkv" \
    -vf "scale=1600:-2:flags=lanczos:out_color_matrix=bt709,format=yuv420p" \
    -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
    -c:v libx264 -preset slow -crf 22 -movflags +faststart docs/demo.mp4
ffmpeg -y -loglevel error -i "$WORK/master.mkv" \
    -vf "fps=15,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=192:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    docs/demo.gif

echo "Wrote docs/demo.mp4, docs/demo.gif, docs/screenshots/ and docs/icon.png"
ls -lh docs/demo.mp4 docs/demo.gif
