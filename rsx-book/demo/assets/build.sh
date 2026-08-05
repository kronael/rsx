#!/usr/bin/env bash
# Render the graphical order-book comparison as MP4 and GIF.
set -Eeuo pipefail
cd "$(dirname "$0")"
W=578
FPS=25

if [[ ! -d frames ]]
then
  echo "no frames/ — run: uv run gen_collage.py" >&2
  exit 1
fi

ffmpeg -y -loglevel error -framerate "$FPS" -i frames/%04d.png \
  -vf "format=yuv420p" -c:v libx264 book-versus.mp4

ffmpeg -y -loglevel error -i book-versus.mp4 \
  -vf "fps=15,scale=${W}:-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse" \
  book-versus.gif
command -v gifsicle >/dev/null && gifsicle -O3 book-versus.gif -o book-versus-opt.gif \
  && mv book-versus-opt.gif book-versus.gif

echo "wrote book-versus.mp4 + book-versus.gif ($(wc -c < book-versus.gif) bytes)"
