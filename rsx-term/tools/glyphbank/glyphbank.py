#!/usr/bin/env python3
"""Glyph bank: rasterize candidate terminal glyphs, measure their visual
characteristics (ink coverage, centroid, quadrant weights, uniformity), and
build a *calibrated* intensity ramp — perceptually even, not the naive
assumption that space/░/▒/▓/█ are 0/25/50/75/100%.

Output (in this dir):
  glyphbank.json     glyph -> metrics
  ramp.txt           the calibrated ramps (intensity + directional)
  contact_sheet.png  labelled grid for visual verification
"""
import json
import os
from PIL import Image, ImageDraw, ImageFont

FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
SIZE = 64
OUT = os.path.dirname(os.path.abspath(__file__))

# Candidate glyphs, grouped.
SHADES = list(" ░▒▓█")
V_EIGHTHS = list("▁▂▃▄▅▆▇█")          # fill from bottom (vertical bar)
H_EIGHTHS = list("▏▎▍▌▋▊▉█")          # fill from left (horizontal bar)
QUAD_HALF = list("▀▄▌▐▖▗▘▝▙▟▛▜▚▞")     # quadrants + halves
BRAILLE = [chr(0x2800 + i) for i in range(256)]  # 0..8 dots, all patterns
MARKERS = list("·•◦∘○●◆◇■□▪▫★☆✦")      # candidate special-state markers
ASCII = [chr(c) for c in range(33, 127)]  # printable ASCII: letters/digits/punct

ALL = SHADES + V_EIGHTHS + H_EIGHTHS + QUAD_HALF + BRAILLE + MARKERS + ASCII

font = ImageFont.truetype(FONT, SIZE)
# Cell box from the full block (it fills the em cell in this font).
cw = int(round(font.getlength("█")))
asc, desc = font.getmetrics()
ch = asc + desc


def metrics(g):
    img = Image.new("L", (cw, ch), 255)  # white bg
    d = ImageDraw.Draw(img)
    d.text((0, 0), g, font=font, fill=0)  # black glyph
    px = img.load()
    total = cw * ch
    ink = 0
    sx = sy = 0.0
    quad = [0, 0, 0, 0]  # TL TR BL BR
    for y in range(ch):
        for x in range(cw):
            v = 255 - px[x, y]          # ink intensity 0..255
            if v > 40:                  # threshold
                ink += 1
                sx += x
                sy += y
                q = (0 if y < ch / 2 else 2) + (0 if x < cw / 2 else 1)
                quad[q] += 1
    cov = ink / total
    if ink:
        cx, cy = sx / ink / cw, sy / ink / ch
        qf = [q / ink for q in quad]
        mean = sum(qf) / 4
        var = sum((q - mean) ** 2 for q in qf) / 4
        uniform = 1 - min(1.0, var ** 0.5 / 0.5)  # 1=even fill, 0=directional
    else:
        cx = cy = 0.5
        qf = [0, 0, 0, 0]
        uniform = 1.0
    return dict(cov=round(cov, 4), cx=round(cx, 3), cy=round(cy, 3),
                quad=[round(q, 3) for q in qf], uniform=round(uniform, 3))


bank = {}
for g in ALL:
    if g == " ":
        bank[g] = dict(cov=0.0, cx=0.5, cy=0.5, quad=[0, 0, 0, 0], uniform=1.0)
    else:
        bank[g] = metrics(g)

with open(f"{OUT}/glyphbank.json", "w") as f:
    json.dump({hex(ord(g)): {"glyph": g, **m} for g, m in bank.items()}, f,
              ensure_ascii=False, indent=1)

# ---- Calibrated intensity ramp: uniform-fill glyphs at even coverage steps.
uniform_pool = [(g, m) for g, m in bank.items()
                if m["uniform"] >= 0.72 and m["cov"] > 0]
uniform_pool.sort(key=lambda gm: gm[1]["cov"])
LEVELS = 12
ramp = [" "]
for i in range(1, LEVELS + 1):
    target = i / LEVELS
    g = min(uniform_pool, key=lambda gm: abs(gm[1]["cov"] - target))[0]
    ramp.append(g)

# de-dupe while preserving order, keep monotone-ish
seen = set()
ramp_final = []
for g in ramp:
    if g not in seen:
        seen.add(g)
        ramp_final.append(g)

# directional sets
bottom = sorted([g for g, m in bank.items() if m["cy"] > 0.62 and m["cov"] > .1],
                key=lambda g: bank[g]["cov"])
left = sorted([g for g, m in bank.items() if m["cx"] < 0.4 and m["cov"] > .1],
              key=lambda g: bank[g]["cov"])

with open(f"{OUT}/ramp.txt", "w") as f:
    f.write("=== measured shade family (naive vs real coverage) ===\n")
    for g in SHADES:
        f.write(f"  {g!r}  cov={bank[g]['cov']:.3f}  uniform={bank[g]['uniform']:.2f}\n")
    f.write("\n=== CALIBRATED intensity ramp (uniform fills, even coverage) ===\n")
    f.write("  " + "".join(ramp_final) + "\n")
    f.write("  " + "  ".join(f"{g}:{bank[g]['cov']:.2f}" for g in ramp_final) + "\n")
    plain_pool = sorted([(g, m) for g, m in bank.items()
                         if (33 <= ord(g) < 127 or g in "░▒▓█") and m["cov"] > 0],
                        key=lambda gm: gm[1]["cov"])
    N = 16
    pr, s2 = [" "], {" "}
    for i in range(1, N + 1):
        g = min(plain_pool, key=lambda gm: abs(gm[1]["cov"] - i / N))[0]
        if g not in s2:
            s2.add(g)
            pr.append(g)
    f.write("\n=== PLAIN fine ramp (letters+shades, universal font, ASCII-art) ===\n")
    f.write("  |" + "".join(pr) + "|\n")
    f.write("  " + "  ".join(f"{g}:{bank[g]['cov']:.2f}" for g in pr) + "\n")
    f.write("\n=== braille density ladder (finer near-uniform texture) ===\n")
    br = sorted([(g, m) for g, m in bank.items() if 0x2800 <= ord(g) <= 0x28ff and m["cov"] > 0],
                key=lambda gm: gm[1]["cov"])
    step = max(1, len(br) // 9)
    for g, m in br[::step]:
        f.write(f"  {g}  cov={m['cov']:.3f}\n")
    f.write("\n=== directional (bottom-weighted, for vertical bars) ===\n  " + "".join(bottom) + "\n")
    f.write("=== directional (left-weighted, for horizontal bars) ===\n  " + "".join(left) + "\n")

# ---- Contact sheet for visual verification.
cols = 16
cell = 44
rows = (len(ALL) + cols - 1) // cols
sheet = Image.new("RGB", (cols * cell, rows * (cell + 12) + 40), "white")
sd = ImageDraw.Draw(sheet)
small = ImageFont.truetype(FONT, 30)
tiny = ImageFont.truetype(FONT, 10)
sd.text((6, 6), f"glyph bank  cell={cw}x{ch}  {len(ALL)} glyphs  (label=coverage)", font=tiny, fill="black")
for i, g in enumerate(ALL):
    r, c = divmod(i, cols)
    x, y = c * cell, r * (cell + 12) + 30
    sd.rectangle([x, y, x + cell - 2, y + cell - 2], outline="#ccc")
    sd.text((x + 8, y + 2), g, font=small, fill="black")
    sd.text((x + 2, y + cell - 10), f"{bank[g]['cov']:.2f}", font=tiny, fill="#0a7")
sheet.save(f"{OUT}/contact_sheet.png")
print("wrote glyphbank.json, ramp.txt, contact_sheet.png")
print("calibrated ramp:", "".join(ramp_final))
