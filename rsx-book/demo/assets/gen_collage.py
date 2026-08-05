#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build the character-medallion intro for the rsx-book benchmark demo."""

import math
import os
from dataclasses import dataclass

from PIL import Image
from PIL import ImageDraw
from PIL import ImageEnhance
from PIL import ImageFont

W = 578
H = 700
FPS = 25
STAGE_SECONDS = 2.6
BG = (4, 8, 6)
PANEL = (13, 23, 18)
RUST = (176, 112, 63)
FG = (236, 230, 216)
VIOLET = (169, 146, 255)
GOLD = (201, 162, 78)
DIM = (143, 134, 114)
GREEN = (34, 245, 161)
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "frames")


@dataclass(frozen=True, slots=True)
class Lane:
    key: str
    name: str
    idea: str
    detail: str
    result: str
    meter: float
    color: tuple[int, int, int]


LANES = [
    Lane(
        "neanderthal",
        "neanderthal",
        "ordered tree",
        "baseline book with\ntree + per-level queues",
        "baseline · pointer-heavy cancel",
        0.25,
        RUST,
    ),
    Lane(
        "engineer",
        "the engineer",
        "+ direct level table",
        "replace tree walks\nwith direct price access",
        "direct levels added",
        0.50,
        VIOLET,
    ),
    Lane(
        "genius",
        "the genius",
        "+ slab handles",
        "stable order handles\nmake cancel direct",
        "direct cancel added",
        0.75,
        GOLD,
    ),
    Lane(
        "gigachad",
        "GIGACHAD",
        "+ Δ-compression",
        "bounded levels + occupancy\nkeep next-price lookup tight",
        "18.6–19.7M fixed-work fills/s equivalent*\n"
        "p50 50.7–53.7 ns · SE ≤0.024 · p99 83.0–111.9 ns",
        1.0,
        GREEN,
    ),
]


def get_font(
    size: int,
    bold: bool = True,
) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    base = "/usr/share/fonts/truetype/dejavu/"
    name = "DejaVuSans-Bold.ttf" if bold else "DejaVuSans.ttf"
    try:
        return ImageFont.truetype(base + name, size)
    except OSError:
        return ImageFont.load_default()


def clamp(value: float) -> float:
    return max(0.0, min(1.0, value))


def ease(value: float) -> float:
    value = clamp(value)
    return 1.0 - (1.0 - value) ** 3


def mix_color(
    start: tuple[int, int, int],
    end: tuple[int, int, int],
    progress: float,
) -> tuple[int, int, int]:
    progress = clamp(progress)
    return (
        int(start[0] + (end[0] - start[0]) * progress),
        int(start[1] + (end[1] - start[1]) * progress),
        int(start[2] + (end[2] - start[2]) * progress),
    )


def load_portrait(key: str, size: int) -> Image.Image:
    for ext in "png", "jpg", "jpeg", "webp":
        path = os.path.join(HERE, f"{key}.{ext}")
        if not os.path.isfile(path):
            continue
        source = Image.open(path).convert("RGBA")
        if key == "engineer":
            source = source.crop(
                (
                    int(source.width * 0.14),
                    0,
                    int(source.width * 0.82),
                    int(source.height * 0.68),
                )
            )
        background = Image.new("RGBA", source.size, BG + (255,))
        portrait = Image.alpha_composite(background, source).convert("RGB")
        if key == "gigachad":
            portrait = ImageEnhance.Brightness(portrait).enhance(1.28)
            portrait = ImageEnhance.Contrast(portrait).enhance(1.12)
        side = min(portrait.size)
        left = (portrait.width - side) // 2
        top = (portrait.height - side) // 2
        portrait = portrait.crop((left, top, left + side, top + side))
        return portrait.resize((size, size), Image.Resampling.LANCZOS)

    portrait = Image.new("RGB", (size, size), PANEL)
    draw = ImageDraw.Draw(portrait)
    draw.rectangle((0, 0, size - 1, size - 1), outline=DIM, width=2)
    draw.text(
        (size // 2, size // 2),
        key,
        fill=DIM,
        font=get_font(14),
        anchor="mm",
    )
    return portrait


def build_medallion(
    lane: Lane,
    size: int,
    angle: float,
    ring: tuple[int, int, int],
) -> Image.Image:
    portrait = load_portrait(lane.key, size)
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).ellipse((2, 2, size - 3, size - 3), fill=255)

    medallion = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    medallion.paste(portrait, (0, 0), mask)
    draw = ImageDraw.Draw(medallion)
    draw.ellipse((2, 2, size - 3, size - 3), outline=ring + (255,), width=4)
    return medallion.rotate(
        angle,
        Image.Resampling.BICUBIC,
        expand=True,
    )


def draw_glow(
    draw: ImageDraw.ImageDraw,
    bounds: tuple[int, int, int, int],
    strength: float,
) -> None:
    if strength <= 0:
        return
    for spread in 14, 10, 6:
        color = mix_color(BG, GREEN, strength * (16 - spread) / 18)
        left, top, right, bottom = bounds
        draw.rounded_rectangle(
            (left - spread, top - spread, right + spread, bottom + spread),
            12 + spread,
            outline=color,
            width=3,
        )


def draw_typed_detail(
    draw: ImageDraw.ImageDraw,
    lane: Lane,
    progress: float,
) -> None:
    typed = clamp((progress - 0.18) / 0.35)
    count = int(len(lane.detail) * typed)
    text = lane.detail[:count]
    if 0 < count < len(lane.detail):
        text += "▌"
    draw.multiline_text(
        (W // 2, 225),
        text,
        fill=FG,
        font=get_font(18, bold=False),
        anchor="mm",
        align="center",
        spacing=9,
    )


def draw_meter(
    image: Image.Image,
    stage_index: int,
    stage_progress: float,
    elapsed: float,
) -> None:
    draw = ImageDraw.Draw(image)
    lane = LANES[stage_index]
    previous_meter = 0.0 if stage_index == 0 else LANES[stage_index - 1].meter
    previous_color = DIM if stage_index == 0 else LANES[stage_index - 1].color
    rise = ease((stage_progress - 0.58) / 0.24)
    meter = previous_meter + (lane.meter - previous_meter) * rise
    color = mix_color(previous_color, lane.color, rise)

    bar_left = 28
    bar_top = 565
    bar_right = W - 28
    bar_bottom = 605
    bar_width = bar_right - bar_left

    final_strength = 0.0
    if stage_index == len(LANES) - 1:
        pulse = 0.55 + 0.45 * math.sin(elapsed * math.tau * 1.8)
        final_strength = rise * pulse
        overshoot = math.sin(math.pi * rise) * 0.07
        meter = min(1.0, meter + overshoot)
    draw_glow(draw, (bar_left, bar_top, bar_right, bar_bottom), final_strength)

    draw.text(
        (bar_left, 538),
        "design progression",
        fill=DIM,
        font=get_font(14),
        anchor="lm",
    )
    draw.text(
        (bar_right, 538),
        "meter never resets",
        fill=DIM,
        font=get_font(12, bold=False),
        anchor="rm",
    )
    draw.rounded_rectangle(
        (bar_left, bar_top, bar_right, bar_bottom),
        10,
        fill=PANEL,
        outline=(39, 55, 47),
        width=2,
    )
    fill_right = bar_left + max(8, int(bar_width * meter))
    draw.rounded_rectangle(
        (bar_left, bar_top, fill_right, bar_bottom),
        10,
        fill=color,
    )

    for index, marker_lane in enumerate(LANES):
        marker_x = bar_left + int(bar_width * marker_lane.meter)
        marker_color = marker_lane.color if index <= stage_index else (55, 66, 59)
        draw.line(
            (marker_x, bar_bottom + 4, marker_x, bar_bottom + 11),
            fill=marker_color,
            width=2,
        )

    if stage_progress >= 0.63:
        result_progress = ease((stage_progress - 0.63) / 0.14)
        result_color = mix_color(BG, lane.color, result_progress)
        result_font = get_font(11 if stage_index == 3 else 15)
        draw.multiline_text(
            (W // 2, 632),
            lane.result,
            fill=result_color,
            font=result_font,
            anchor="mm",
            align="center",
            spacing=4,
        )


def draw_medallions(
    image: Image.Image,
    stage_index: int,
    stage_progress: float,
    elapsed: float,
) -> None:
    draw = ImageDraw.Draw(image)
    centers = [76, 218, 360, 502]
    for index in range(stage_index + 1):
        lane = LANES[index]
        active = index == stage_index
        entry = ease(stage_progress / 0.22) if active else 1.0
        base_size = 106 if index == len(LANES) - 1 else 88
        size = max(8, int(base_size * entry))
        angle = -22.0 * (1.0 - entry) if active else 0.0
        ring = lane.color if active else mix_color(BG, lane.color, 0.42)

        if active and index == len(LANES) - 1 and stage_progress > 0.58:
            pulse = 0.5 + 0.5 * math.sin(elapsed * math.tau * 1.8)
            radius = base_size // 2 + int(7 * pulse)
            draw.ellipse(
                (
                    centers[index] - radius,
                    455 - radius,
                    centers[index] + radius,
                    455 + radius,
                ),
                outline=mix_color(BG, GREEN, 0.45 + 0.4 * pulse),
                width=3,
            )

        medallion = build_medallion(lane, size, angle, ring)
        x = centers[index] - medallion.width // 2
        y = 455 - medallion.height // 2 + int(35 * (1.0 - entry))
        image.paste(medallion, (x, y), medallion)
        draw.text(
            (centers[index], 519),
            str(index + 1),
            fill=ring,
            font=get_font(12),
            anchor="mm",
        )


def build_frame(frame_index: int) -> Image.Image:
    image = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(image)
    elapsed = frame_index / FPS
    stage_float = elapsed / STAGE_SECONDS
    stage_index = min(int(stage_float), len(LANES) - 1)
    stage_progress = clamp(stage_float - stage_index)
    lane = LANES[stage_index]

    draw.text(
        (W // 2, 40),
        "one book. four designs.",
        fill=GOLD,
        font=get_font(22),
        anchor="mm",
    )
    draw.text(
        (W // 2, 67),
        "each player adds to the same persistent design stack",
        fill=DIM,
        font=get_font(12, bold=False),
        anchor="mm",
    )

    draw.rounded_rectangle(
        (28, 92, W - 28, 342),
        10,
        fill=PANEL,
        outline=mix_color(PANEL, lane.color, 0.45),
        width=2,
    )
    draw.text(
        (W // 2, 126),
        lane.name,
        fill=lane.color,
        font=get_font(17),
        anchor="mm",
    )
    draw.text(
        (W // 2, 171),
        lane.idea,
        fill=FG,
        font=get_font(28),
        anchor="mm",
    )
    draw_typed_detail(draw, lane, stage_progress)

    draw_medallions(image, stage_index, stage_progress, elapsed)
    draw_meter(image, stage_index, stage_progress, elapsed)
    footer = "concept stages · measured result on final card"
    if stage_index == len(LANES) - 1 and stage_progress >= 0.63:
        footer = "*p50 reciprocal · one partial fill · 100K–10M resting orders"
    draw.text(
        (W // 2, 674),
        footer,
        fill=DIM,
        font=get_font(11, bold=False),
        anchor="mm",
    )
    return image


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    for name in os.listdir(OUT):
        if name.endswith(".png"):
            os.remove(os.path.join(OUT, name))

    frame_count = int(FPS * STAGE_SECONDS * len(LANES))
    for frame_index in range(frame_count):
        frame = build_frame(frame_index)
        frame.save(os.path.join(OUT, f"{frame_index:04d}.png"))
    print(f"wrote {frame_count} frames to frames/")


if __name__ == "__main__":
    main()
