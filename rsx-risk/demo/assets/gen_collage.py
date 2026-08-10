#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build the character-medallion intro for the rsx-risk benchmark demo.

Four design stages for the pre-trade gate, then a measured final card.
Numbers on the final card come from reports/20260530_component-benches.md —
never edit them here without a new dated run.
"""

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
CONCLUSION_SECONDS = 4.4
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
        "ask the database",
        "read positions from Postgres\non every single order",
        "baseline · a network hop per order",
        0.25,
        RUST,
    ),
    Lane(
        "engineer",
        "the engineer",
        "+ hold it in RAM",
        "shard owns its users;\nno round trip on the path",
        "round trip removed",
        0.50,
        VIOLET,
    ),
    Lane(
        "genius",
        "the genius",
        "+ exposure index",
        "touch only the users a\nprice move can actually reach",
        "rescan removed",
        0.75,
        GOLD,
    ),
    Lane(
        "gigachad",
        "GIGACHAD",
        "+ pinned, zero-alloc",
        "no runtime, no heap, no lock\non the order critical path",
        "design stack complete",
        1.0,
        GREEN,
    ),
]

# reports/20260530_component-benches.md — Criterion, single core.
COMPONENTS = [
    ("pre-trade gate", "~110 ns"),
    ("apply a fill", "~3.6 ns"),
    ("BBO -> index", "~5 ns"),
    ("exposure lookup", "~1.6 ns"),
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
        draw.multiline_text(
            (W // 2, 632),
            lane.result,
            fill=result_color,
            font=get_font(14),
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


def draw_conclusion_meter(
    image: Image.Image,
    progress: float,
    elapsed: float,
) -> None:
    draw = ImageDraw.Draw(image)
    bar_left = 28
    bar_top = 565
    bar_right = W - 28
    bar_bottom = 605
    bar_width = bar_right - bar_left
    rise = ease((progress - 0.42) / 0.22)
    # 110 ns spent against a 5 us budget: the bar is headroom, not usage.
    used_ratio = 110 / 5000
    headroom_x = bar_left + int(bar_width * (1.0 - used_ratio) * rise)
    pulse = 0.6 + 0.4 * math.sin(elapsed * math.tau * 1.5)

    draw_glow(
        draw,
        (bar_left, bar_top, bar_right, bar_bottom),
        rise * pulse,
    )
    draw.text(
        (bar_left, 538),
        "budget headroom · 5 µs per order",
        fill=DIM,
        font=get_font(13),
        anchor="lm",
    )
    draw.text(
        (bar_right, 538),
        "110 ns used",
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
    if rise > 0:
        draw.rounded_rectangle(
            (bar_left, bar_top, max(bar_left + 8, headroom_x), bar_bottom),
            10,
            fill=GREEN,
        )

    result_color = mix_color(BG, GREEN, ease((progress - 0.58) / 0.16))
    draw.text(
        (W // 2, 629),
        "45× under the per-order budget",
        fill=result_color,
        font=get_font(15),
        anchor="mm",
    )
    draw.text(
        (W // 2, 651),
        "Criterion aggregate · single core · p99 not yet measured",
        fill=DIM,
        font=get_font(10, bold=False),
        anchor="mm",
    )
    draw.text(
        (W // 2, 676),
        "reports/20260530_component-benches.md",
        fill=DIM,
        font=get_font(10, bold=False),
        anchor="mm",
    )


def build_conclusion_frame(progress: float, elapsed: float) -> Image.Image:
    image = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(image)
    reveal = ease(progress / 0.18)

    draw.text(
        (W // 2, 40),
        "game over. measured result.",
        fill=mix_color(BG, GREEN, reveal),
        font=get_font(22),
        anchor="mm",
    )
    draw.text(
        (W // 2, 67),
        "RAM shard + exposure index + zero-alloc hot path",
        fill=mix_color(BG, DIM, reveal),
        font=get_font(12, bold=False),
        anchor="mm",
    )
    draw.rounded_rectangle(
        (28, 92, W - 28, 342),
        10,
        fill=PANEL,
        outline=mix_color(PANEL, GREEN, 0.65 * reveal),
        width=2,
    )
    draw.text(
        (W // 2, 124),
        "EVERY ORDER PAYS THIS",
        fill=mix_color(BG, GREEN, reveal),
        font=get_font(16),
        anchor="mm",
    )
    draw.text(
        (W // 2, 148),
        "per-order risk work on the critical path",
        fill=mix_color(BG, DIM, reveal),
        font=get_font(11, bold=False),
        anchor="mm",
    )

    draw.line((55, 172, W - 55, 172), fill=mix_color(BG, DIM, reveal), width=1)

    for row_index, (label, value) in enumerate(COMPONENTS):
        row_reveal = ease((progress - 0.13 - row_index * 0.09) / 0.14)
        row_y = 190 + row_index * 30
        draw.text(
            (130, row_y),
            label,
            fill=mix_color(BG, FG, row_reveal),
            font=get_font(14, bold=False),
            anchor="lm",
        )
        draw.text(
            (W - 130, row_y),
            value,
            fill=mix_color(BG, GREEN if row_index == 0 else FG, row_reveal),
            font=get_font(14),
            anchor="rm",
        )

    draw.text(
        (W // 2, 327),
        "the gate that keeps one trader from sinking the exchange",
        fill=mix_color(BG, GOLD, ease((progress - 0.42) / 0.15)),
        font=get_font(12),
        anchor="mm",
    )
    draw_medallions(image, len(LANES) - 1, 1.0, elapsed)
    draw_conclusion_meter(image, progress, elapsed)
    return image


def build_frame(frame_index: int) -> Image.Image:
    elapsed = frame_index / FPS
    design_duration = STAGE_SECONDS * len(LANES)
    if elapsed >= design_duration:
        conclusion_progress = clamp(
            (elapsed - design_duration) / CONCLUSION_SECONDS
        )
        return build_conclusion_frame(conclusion_progress, elapsed)

    image = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(image)
    stage_float = elapsed / STAGE_SECONDS
    stage_index = min(int(stage_float), len(LANES) - 1)
    stage_progress = clamp(stage_float - stage_index)
    lane = LANES[stage_index]

    draw.text(
        (W // 2, 40),
        "one risk check. four designs.",
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
    draw.text(
        (W // 2, 674),
        "concept stages · measured result on final card",
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

    frame_count = int(
        FPS * (STAGE_SECONDS * len(LANES) + CONCLUSION_SECONDS)
    )
    for frame_index in range(frame_count):
        frame = build_frame(frame_index)
        frame.save(os.path.join(OUT, f"{frame_index:04d}.png"))
    print(f"wrote {frame_count} frames to frames/")


if __name__ == "__main__":
    main()
