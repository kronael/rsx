#!/usr/bin/env bash
# Composite the collage intro (frames/, from gen_collage.py) into the
# graphics-only playground artifact: risk-versus.mp4 + risk-versus.gif.
# Re-run after dropping character images + `uv run gen_collage.py`.
set -euo pipefail
cd "$(dirname "$0")"
W=578; H=700; FPS=25

[ -d frames ] || { echo "no frames/ — run: uv run gen_collage.py" >&2; exit 1; }

ffmpeg -y -loglevel error -framerate "$FPS" -i frames/%04d.png \
  -vf "format=yuv420p" -c:v libx264 risk-versus.mp4

ffmpeg -y -loglevel error -i risk-versus.mp4 \
  -vf "fps=15,scale=${W}:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=64[p];[b][p]paletteuse=dither=bayer" \
  risk-versus.gif
command -v gifsicle >/dev/null && gifsicle -O3 risk-versus.gif -o risk-versus-opt.gif \
  && mv risk-versus-opt.gif risk-versus.gif

echo "wrote risk-versus.mp4 + risk-versus.gif ($(wc -c < risk-versus.gif) bytes)"
