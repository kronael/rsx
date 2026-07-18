#!/usr/bin/env python3
"""Stitch the exchange-walkthrough captures into demo/exchange.gif.

Inputs (produced by exchange-walkthrough.sh in its output dir): browser
PNGs named NN-*.png, plus agg-rendered terminal GIFs (trace.gif,
walpeek.gif, trio.gif) from which representative frames are pulled.
Frames are ordered by their numeric name prefix, letterboxed onto a
960x600 canvas, and written with per-frame durations.
"""
import glob
import os
import sys

from PIL import Image
from PIL import ImageSequence

CANVAS = (960, 600)
BG: tuple[int, int, int] = (4, 8, 6)  # #040806, the Ayam Cemani near-black

# terminal-cast frames to extract: (gif stem, fraction-of-frames, out name)
CAST_FRAMES = [
    ("trace", 0.22, "11-trace-sent"),
    ("trace", 0.55, "12-trace-filled"),
    ("trace", 1.00, "13-trace-stamps"),
    ("walpeek", 1.00, "22-wal-peek"),
    ("trio", 0.35, "31-trio-a"),
    ("trio", 0.65, "32-trio-b"),
    ("trio", 0.95, "33-trio-c"),
]

DURATION_MS = {
    "00-title": 3000,
    "01-topology": 3600,
    "02-topology-detail": 3000,
    "10-journey": 2600,
    "11-trace-sent": 1300,
    "12-trace-filled": 1300,
    "13-trace-stamps": 4600,
    "20-tape": 2600,
    "21-wal-page": 3200,
    "22-wal-peek": 3800,
    "30-scale": 2400,
    "31-trio-a": 1100,
    "32-trio-b": 1100,
    "33-trio-c": 2000,
    "40-end": 3000,
}
DEFAULT_MS = 2500


def cast_frame(path: str, fraction: float) -> Image.Image:
    with Image.open(path) as gif:
        frames = [f.convert("RGB") for f in ImageSequence.Iterator(gif)]
    index = min(len(frames) - 1, max(0, round((len(frames) - 1) * fraction)))
    return frames[index]


def letterbox(image: Image.Image) -> Image.Image:
    scale = min(CANVAS[0] / image.width, CANVAS[1] / image.height)
    size = (round(image.width * scale), round(image.height * scale))
    canvas = Image.new("RGB", CANVAS, BG)
    resized = image.resize(size, Image.Resampling.LANCZOS)
    canvas.paste(resized, ((CANVAS[0] - size[0]) // 2,
                           (CANVAS[1] - size[1]) // 2))
    return canvas


def main() -> int:
    out_dir = sys.argv[1]
    gif_path = sys.argv[2]

    frames: dict[str, Image.Image] = {}
    for png in glob.glob(f"{out_dir}/[0-9]*.png"):
        name = os.path.splitext(os.path.basename(png))[0]
        frames[name] = letterbox(Image.open(png).convert("RGB"))
    for stem, fraction, name in CAST_FRAMES:
        path = f"{out_dir}/{stem}.gif"
        if os.path.exists(path):
            frames[name] = letterbox(cast_frame(path, fraction))
        else:
            print(f"  missing cast {path} -- skipping {name}")

    names = sorted(frames)
    if not names:
        raise SystemExit("no frames found")
    images = [frames[n].convert("P", palette=Image.Palette.ADAPTIVE, colors=128)
              for n in names]
    durations = [DURATION_MS.get(n, DEFAULT_MS) for n in names]
    images[0].save(gif_path, save_all=True, append_images=images[1:],
                   duration=durations, loop=0, optimize=True)
    print(f"  {gif_path}: {len(names)} frames, "
          f"{os.path.getsize(gif_path) // 1024} KB")
    for n in names:
        print(f"    {n}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
