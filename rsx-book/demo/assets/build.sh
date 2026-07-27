#!/usr/bin/env bash
# Composite the collage intro (frames/, from gen_collage.py) with the terminal
# bench GIF into one portrait clip: book-versus.mp4 + book-versus.gif.
# Re-run after dropping real character images + `uv run gen_collage.py`.
set -euo pipefail
cd "$(dirname "$0")"
W=578; H=700; FPS=25
BENCH=../book-live-opt.gif

[ -d frames ] || { echo "no frames/ — run: uv run gen_collage.py" >&2; exit 1; }

# 1. collage intro (PNG sequence -> h264)
ffmpeg -y -loglevel error -framerate "$FPS" -i frames/%04d.png \
  -vf "format=yuv420p" -c:v libx264 intro.mp4

# 2. terminal bench GIF -> h264 at matching geometry/fps
ffmpeg -y -loglevel error -i "$BENCH" \
  -vf "scale=${W}:${H}:flags=lanczos,fps=${FPS},format=yuv420p" \
  -c:v libx264 bench.mp4

# 3. hard cut: intro then bench
ffmpeg -y -loglevel error -i intro.mp4 -i bench.mp4 \
  -filter_complex "[0:v][1:v]concat=n=2:v=1:a=0[v]" -map "[v]" \
  -c:v libx264 -pix_fmt yuv420p book-versus.mp4

# 4. postable GIF (palette for clean colors) + optimize
ffmpeg -y -loglevel error -i book-versus.mp4 \
  -vf "fps=15,scale=${W}:-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse" \
  book-versus.gif
command -v gifsicle >/dev/null && gifsicle -O3 book-versus.gif -o book-versus-opt.gif \
  && mv book-versus-opt.gif book-versus.gif

rm -f intro.mp4 bench.mp4
echo "wrote book-versus.mp4 + book-versus.gif ($(wc -c < book-versus.gif) bytes)"
