#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Collage-animation intro for the book demo. Composites 4 CHARACTER IMAGES
(you provide them; labeled placeholders are generated if missing) into an
animated versus scoreboard, then hands off to the terminal bench.

Drop real images here as: neanderthal.(png|jpg), engineer.*, genius.*,
gigachad.* — then re-run:  uv run gen_collage.py
Outputs frames/ (PNG sequence) consumed by build.sh (ffmpeg -> mp4/gif +
concat with ../book-live-opt.gif).

I only supply MOTION + COMPOSITION here — the character art is yours.
"""
import os
from PIL import Image, ImageDraw, ImageFont

W, H = 578, 700                      # portrait, matches the terminal GIF width
FPS = 25
BG = (4, 8, 6)                       # Cemani page
PANEL = (13, 23, 18)
RUST = (176, 112, 63)
FG = (236, 230, 216)
TEAL = (87, 176, 163)
GOLD = (201, 162, 78)
DIM = (143, 134, 114)
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "frames")

# lane: key, display name, impl, bar-fill fraction (short=fast=good), accent
LANES = [
    ("neanderthal", "neanderthal", "BTreeMap",            1.00, RUST),
    ("engineer",    "the engineer", "flat array",         0.64, FG),
    ("genius",      "the genius",   "slab arena",         0.38, FG),
    ("gigachad",    "GIGACHAD",     "slab + Δ-compress",  0.14, TEAL),
]

def font(sz, bold=True):
    base = "/usr/share/fonts/truetype/dejavu/"
    name = "DejaVuSans-Bold.ttf" if bold else "DejaVuSans.ttf"
    try:
        return ImageFont.truetype(base + name, sz)
    except Exception:
        return ImageFont.load_default()

def load_or_placeholder(key, size):
    for ext in ("png", "jpg", "jpeg", "webp"):
        p = os.path.join(HERE, f"{key}.{ext}")
        if os.path.isfile(p):
            src = Image.open(p).convert("RGBA")       # emoji/art may be transparent
            bg = Image.new("RGBA", src.size, BG + (255,))
            im = Image.alpha_composite(bg, src).convert("RGB")
            # cover-crop to square
            s = min(im.size)
            im = im.crop(((im.width - s) // 2, (im.height - s) // 2,
                          (im.width - s) // 2 + s, (im.height - s) // 2 + s))
            return im.resize((size, size)), True
    # labeled placeholder box
    ph = Image.new("RGB", (size, size), PANEL)
    d = ImageDraw.Draw(ph)
    d.rectangle([0, 0, size - 1, size - 1], outline=DIM, width=2)
    d.text((size // 2, size // 2 - 10), key, fill=DIM, font=font(15),
           anchor="mm")
    d.text((size // 2, size // 2 + 12), "drop image", fill=(90, 84, 72),
           font=font(11), anchor="mm")
    return ph, False

def ease(t):
    return 1 - (1 - t) ** 3

IMG = 96
def frame(i, N):
    im = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(im)
    p = i / N
    # header
    if p > 0.02:
        a = ease(min(1, p / 0.12))
        d.text((28, int(40 - 14 * (1 - a))), "build an order book — who wins?",
               fill=GOLD, font=font(20), anchor="lm")
    top0, rowh = 96, 138
    for li, (key, name, impl, frac, col) in enumerate(LANES):
        start = 0.14 + li * 0.16          # staggered reveal
        lt = (p - start) / 0.16
        if lt <= 0:
            continue
        a = ease(min(1, lt))
        y = top0 + li * rowh
        dx = int(-40 * (1 - a))           # slide in from left
        img, real = load_or_placeholder(key, IMG)
        im.paste(img, (28 + dx, y))
        d.text((28 + IMG + 20 + dx, y + 8), name, fill=col, font=font(20),
               anchor="lm")
        d.text((28 + IMG + 20 + dx, y + 34), impl, fill=DIM, font=font(14),
               anchor="lm")
        # scoreboard bar (fills with easing), short = fast
        bx, by, bw, bh = 28 + IMG + 20 + dx, y + 58, 300, 22
        d.rounded_rectangle([bx, by, bx + bw, by + bh], 4, fill=PANEL)
        fillw = int(bw * frac * a)
        d.rounded_rectangle([bx, by, bx + max(6, fillw), by + bh], 4, fill=col)
        if key == "gigachad" and a > 0.7:
            d.text((bx + bw + 14, by + bh // 2), "★", fill=GOLD, font=font(26),
                   anchor="lm")
            d.text((28 + IMG + 20 + dx, y + 92), "~70 ns · flat to 10 M — wins",
                   fill=GOLD, font=font(13), anchor="lm")
        elif key == "neanderthal" and a > 0.7:
            d.text((28 + IMG + 20 + dx, y + 92), "1.5–2× slower · grows with depth",
                   fill=DIM, font=font(13), anchor="lm")
    # hand-off line
    if p > 0.9:
        a = ease(min(1, (p - 0.9) / 0.1))
        d.text((W // 2, H - 40), "now — the real bench ↓",
               fill=GOLD, font=font(17), anchor="mm")
    return im

def main():
    os.makedirs(OUT, exist_ok=True)
    for f in os.listdir(OUT):
        if f.endswith(".png"):
            os.remove(os.path.join(OUT, f))
    N = int(FPS * 4.2)                    # ~4.2s intro
    real = sum(load_or_placeholder(k, 8)[1] for k, *_ in LANES)
    for i in range(N):
        frame(i, N).save(os.path.join(OUT, f"{i:04d}.png"))
    print(f"wrote {N} frames to frames/  ({real}/4 real images, "
          f"{4 - real} placeholders)")

main()
