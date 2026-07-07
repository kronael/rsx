#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["rich"]
# ///
"""Terminal pitch for rsx-cast.

Two acts:
  1. Narrative -- FLOWS like a real terminal session. Text is typed to stdout
     and stays (no clear, no re-center between claims); it fills top-down the
     way a real terminal does. This is the credibility premise -- it reads as
     a session, not a slideshow.
  2. Benchmark -- the ONE screen clear. Opens on a weighty centered callout
     ("See for yourself."), then reveals the numbers, then the call to action.
     Rendered via a single rich.Live region so callout -> results -> CTA swap
     in place with no further clears.

Recorded via: asciinema rec --overwrite --cols 44 --rows 12 -c "uv run pitch.py" cast-live.cast
Colors: the "Cemani" palette (see CLAUDE.md, "Palette").
"""
import random
import sys
import time

from rich.align import Align
from rich.console import Console
from rich.console import Group
from rich.padding import Padding
from rich.panel import Panel
from rich.table import Table
from rich.live import Live
from rich.text import Text

# "Cemani" palette -- sampled from the black-rooster-in-spring photo (see
# CLAUDE.md, "Palette"): iridescent teal sheen, warm olive-gold bokeh, moss
# grass, earth brown, near-black warm body. Hues faithful to the image,
# lifted to UI-legible brightness on a warm-dark base (agg --theme monokai;
# NOT gruvbox-dark -- agg renders that with gruvbox's LIGHT cream bg).
TEAL = "#57b0a3"  # iridescent feather sheen -- the signature; best/tied protocols
GOLD = "#c9a24e"  # warm olive-gold bokeh -- primary accent, borders, CTA
MOSS = "#9aad4c"  # spring grass green
RUST = "#b0703f"  # warm earth / tree-trunk brown -- the "cost"/worst
FG = "#ece6d8"    # warm off-white highlight -- body text
DIM = "#8f8672"   # muted warm grey-brown -- captions
# comparison gradient: teal best -> gold middle -> rust worst

WIDTH = 44
# Half-height terminal, rendered at font-size 28 (Makefile) -- fewer, denser
# rows: same-ish GIF width, ~half the height, ~40% more pixels per glyph.
# Act 1 scrolls naturally (real terminals scroll); act 2 fits 12 rows exactly.
ROWS = 12
CURSOR = "█"
# Reading pace, not typing-demo pace: ~45 ms/char ≈ 22 chars/s ≈ 250 wpm,
# so a viewer reads along as it types instead of chasing the cursor.
TYPE_SPEED = 0.045
BLINK = 0.4

console = Console(width=WIDTH, color_system="truecolor", force_terminal=True)
out = sys.stdout


def _sgr(hex_color):
    r, g, b = int(hex_color[1:3], 16), int(hex_color[3:5], 16), int(hex_color[5:7], 16)
    return f"\x1b[1;38;2;{r};{g};{b}m"


def type_flow(segments):
    """Type styled text at the current terminal position, char by char, and
    LEAVE it there -- flowing, no clear. The real terminal cursor (shown by
    the caller) trails the text naturally, exactly like live typing. Organic
    pacing: jittered per-char delay + a real stop at sentence-enders."""
    reset = "\x1b[0m"
    prev = ""
    for text, color in segments:
        pre = _sgr(color)
        for ch in text:
            if ch == "\n":
                out.write("\n")
                out.flush()
                # a blank line is a paragraph break -- give the reader a beat
                time.sleep(0.8 if prev == "\n" else 0.05)
                prev = ch
                continue
            out.write(pre + ch + reset)
            out.flush()
            delay = TYPE_SPEED * random.uniform(0.6, 1.4)
            if ch in ".!?":
                delay += random.uniform(0.30, 0.55)  # let the sentence land
            prev = ch
            time.sleep(delay)


def fill(inner, inner_h):
    """Vertically center `inner` in a constant ROWS-tall block, so the Live
    region never changes height as callout -> results -> CTA swap in place."""
    top = max((ROWS - inner_h) // 2, 0)
    bot = max(ROWS - inner_h - top, 0)
    return Padding(inner, (top, 0, bot, 0))


# ── Act 1: flowing narrative (no clear, real cursor trails) ──────────────────

# Word-level meaning colors, not whole-sentence blocks: speed words TEAL,
# durability/brand words GOLD, costs RUST, connective prose FG.
NARRATIVE = [
    ("rsx-cast", GOLD),
    (" — move every byte ", FG), ("fast", TEAL), (",\n", FG),
    ("never lose one", GOLD), (".\n\n", FG),
    ("Total reliability ", FG), ("costs latency", RUST), (":\n", FG),
    ("a broker", RUST), (" adds hops, and ", FG),
    ("fsync on\nevery byte", RUST), (" costs ", FG),
    ("milliseconds", RUST), (" —\na hundred times the network hop.\n\n", FG),
    ("If your servers never all die at\nonce, ", TEAL),
    ("don't pay for it", GOLD), (".\n\n", FG),
    ("rsx-cast", GOLD), (" is as ", FG), ("fast", TEAL),
    (" as it goes,\nand as ", FG), ("reliable", TEAL),
    (" as it gets.\nHow?\n\n", FG),
    ("By making it ", FG), ("minimal", TEAL), (". ", FG),
    ("Multicast", GOLD), (",\nnot pub/sub. The ", FG),
    ("same bytes", TEAL), (" on the\nwire, on disk, on replay. ", FG),
    ("NAK", GOLD), ("\nrecovers a live gap; the ", FG),
    ("batched\nWAL", GOLD), (" is the fallback — off the\ncritical send path.\n\n", FG),
    ("Ties raw UDP. Beats TCP and QUIC.", GOLD),
    ("\n", FG),
]

out.write("\x1b[?25h")  # show the real cursor; it trails the typing
out.write("\n")
out.flush()
type_flow(NARRATIVE)
time.sleep(1.4)  # let the last claim land before we cut away
out.write("\x1b[?25l")  # hide it again for the controlled Live section
out.flush()

# ── Act 2: the ONE clear, then callout -> results -> CTA (all in-place) ──────

# label, us, color, is_best -- all cited from compare/README.md; cast_rtt_bench's
# live re-run is blocked by CAST-RTT-BENCH-HANGS-AFTER-SEND-REMOVAL (BUGS.md).
PROTOS = [
    ("cast", 9.3, TEAL, True),
    ("udp ", 9.9, TEAL, False),
    ("tcp ", 14.0, GOLD, False),
    ("quic", 37.0, RUST, False),
]
FRAMES = 200  # 200 * 0.05s = 10s count-up


def callout(cursor_on):
    c = CURSOR if cursor_on else " "
    line = Text.from_markup(f"[bold {GOLD}]See for yourself.[/bold {GOLD}] {c}")
    return fill(Align.center(line), 1)


def results(frame):
    table = Table.grid(padding=(0, 1))
    table.add_column(width=4)
    table.add_column(width=11, justify="right")
    table.add_column(width=7, justify="right")
    table.add_column(width=2)
    t = 1 - (1 - frame / FRAMES) ** 2  # ease-out
    landed = t >= 1.0
    for label, us, color, best in PROTOS:
        cur = (1_000_000 / us) * min(t, 1.0)
        mark = f"[bold {TEAL}]★[/bold {TEAL}]" if (best and landed) else ""
        table.add_row(
            f"[bold]{label}[/bold]",
            f"[{color}]{cur:,.0f}/s[/{color}]",
            f"[{DIM}]{us:>4.1f}µs[/{DIM}]",
            mark,
        )
    scope = f"[{DIM}]2 pinned cores, 128B, loopback[/{DIM}]"
    # compact: no blank rows, no vertical padding, no caption -- the whole
    # panel is 2 borders + scope + 4 rows = 7 rows, fits ROWS=12.
    # (derivation/citation notes live in demo/CLAUDE.md, not on screen.)
    panel = Panel(
        Group(scope, table),
        title="[bold]throughput · latency[/bold]",
        title_align="left",
        border_style=GOLD,
        width=WIDTH - 2,
        padding=(0, 2),
    )
    return fill(panel, 7)


def cta(cursor_on):
    c = CURSOR if cursor_on else " "
    lines = Text.from_markup(
        f"[bold {GOLD}]Read the code.[/bold {GOLD}]\n"
        f"[bold {FG}]github.com/kronael/rsx[/bold {FG}] {c}"
    )
    return fill(Align.center(lines), 2)


console.clear()
console.show_cursor(False)
try:
    with Live(callout(True), console=console, auto_refresh=False,
              vertical_overflow="crop") as live:
        # weighty pause on the callout
        t0 = time.monotonic()
        while time.monotonic() - t0 < 2.6:
            on = int((time.monotonic() - t0) / BLINK) % 2 == 0
            live.update(callout(on), refresh=True)
            time.sleep(0.1)
        # numbers climb in (bigger = better, intuitive)
        for frame in range(1, FRAMES + 1):
            live.update(results(frame), refresh=True)
            time.sleep(0.05)
        # hold on the landed result, blinking -- long enough to actually read
        t0 = time.monotonic()
        while time.monotonic() - t0 < 15.0:
            on = int((time.monotonic() - t0) / BLINK) % 2 == 0
            live.update(results(FRAMES), refresh=True)
            time.sleep(0.1)
        # one clear call to action
        t0 = time.monotonic()
        while time.monotonic() - t0 < 5.0:
            on = int((time.monotonic() - t0) / BLINK) % 2 == 0
            live.update(cta(on), refresh=True)
            time.sleep(0.1)
finally:
    console.show_cursor(True)
