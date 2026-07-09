#!/usr/bin/env python3
"""Tiny asciinema-to-GIF renderer for the RSX local demo.

This intentionally covers the demo's simple terminal output instead of
replacing agg. It keeps the recording reproducible when agg is absent.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def strip_ansi(s: str) -> str:
    return ANSI.sub("", s.replace("\r", ""))


def font(size: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    for path in (
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationMono-Regular.ttf",
    ):
        if Path(path).exists():
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def push_text(lines: list[str], text: str, rows: int) -> None:
    for part in strip_ansi(text).split("\n"):
        if part:
            lines.append(part[:200])
        else:
            lines.append("")
    del lines[:-rows]


def frame(lines: list[str], cols: int, rows: int, fnt) -> Image.Image:
    char_w = max(8, int(fnt.getlength("M")))
    line_h = int(fnt.getbbox("M")[3] * 1.45)
    pad = 18
    img = Image.new("RGB", (cols * char_w + pad * 2, rows * line_h + pad * 2), "#040806")
    draw = ImageDraw.Draw(img)
    y = pad
    for line in lines[-rows:]:
        color = "#d6e0eb"
        if line.startswith("$ "):
            color = "#7c9389"
        elif line.startswith(("RSX ", "1. ", "2. ", "3. ", "4. ", "5. ", "Demo")):
            color = "#5dffb9"
        elif "FAIL" in line or "missing" in line:
            color = "#ff7b7b"
        draw.text((pad, y), line[:cols], font=fnt, fill=color)
        y += line_h
    return img


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cast")
    ap.add_argument("gif")
    ap.add_argument("--cols", type=int, default=88)
    ap.add_argument("--rows", type=int, default=26)
    args = ap.parse_args()

    lines: list[str] = []
    frames: list[Image.Image] = []
    durations: list[int] = []
    fnt = font(18)
    last_t = 0.0

    with open(args.cast, encoding="utf-8") as fh:
        fh.readline()
        for raw in fh:
            t, kind, data = json.loads(raw)
            if kind != "o":
                continue
            delay = max(80, min(900, int((t - last_t) * 1000)))
            push_text(lines, data, args.rows)
            if not any(line.strip() for line in lines):
                last_t = t
                continue
            frames.append(frame(lines, args.cols, args.rows, fnt))
            durations.append(delay)
            last_t = t

    if not frames:
        raise SystemExit("empty cast")
    durations[-1] = 2200
    frames[0].save(
        args.gif,
        save_all=True,
        append_images=frames[1:],
        duration=durations,
        loop=0,
        optimize=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
