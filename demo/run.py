#!/usr/bin/env python3
"""Driven demo for the GIF: three live RSX order books, side by side, moving.

Follows the AUTHORITATIVE marketdata WS (ws://127.0.0.1:8180) through
`rsx-cli md follow --json`, so it shows the real exchange book (the /api/bbo
dashboard path is a separate, flaky reconstruction). The RSX system must be up
(`make demo`) and `target/debug/rsx-cli` built.

Run from this directory:
    python3 run.py
"""
import asyncio
import json
import os
import sys
import time

CLI = os.path.join(os.path.dirname(__file__), "..", "target", "debug", "rsx-cli")
MD = "ws://127.0.0.1:8180"
TRIO = [(10, "PENGU", 6), (3, "SOL", 4), (1, "BTC", 2)]
FRAMES = 18
LEVELS = 4
COLW = 30

DIM = "\x1b[38;5;244m"
GRN = "\x1b[38;5;42m"
RED = "\x1b[38;5;203m"
CYN = "\x1b[38;5;80m"
WHT = "\x1b[38;5;252m"
RST = "\x1b[0m"
CLR = "\x1b[H\x1b[2J"

books = {sid: {"bids": [], "asks": []} for sid, _, _ in TRIO}
last_trade = {}


def apply_frame(f):
    kind = f["kind"]
    sid = f.get("symbol_id")
    if sid not in books:
        return
    book = books[sid]
    if kind == "l2_snapshot":
        books[sid] = {
            "bids": [{"px": px, "qty": qty} for px, qty, _ in f["bids"]],
            "asks": [{"px": px, "qty": qty} for px, qty, _ in f["asks"]],
        }
    elif kind == "l2_delta":
        key = "bids" if f["side"] == 0 else "asks"
        set_level(book, key, f["px"], f["qty"])
    elif kind == "bbo":
        # authoritative touch: drop crossed levels, upsert the top
        if f["ask_px"]:
            book["bids"] = [x for x in book["bids"] if x["px"] < f["ask_px"]]
        if f["bid_px"]:
            book["asks"] = [x for x in book["asks"] if x["px"] > f["bid_px"]]
        if f["bid_px"]:
            set_level(book, "bids", f["bid_px"], f["bid_qty"])
        if f["ask_px"]:
            set_level(book, "asks", f["ask_px"], f["ask_qty"])
    elif kind == "trade":
        last_trade[sid] = (f["px"], f["qty"])


def set_level(book, key, px, qty):
    levels = book[key]
    if qty == 0:
        book[key] = [x for x in levels if x["px"] != px]
        return
    hit = next((x for x in levels if x["px"] == px), None)
    if hit:
        hit["qty"] = qty
    else:
        levels.append({"px": px, "qty": qty})
    levels.sort(key=lambda x: x["px"], reverse=key == "bids")


def fmt_px(px, dec):
    return f"{px / 10**dec:,.{dec}f}"


def bar(qty, unit):
    return "▎" * max(1, min(int(qty / unit) if unit else 1, 12))


def book_lines(sid, name, dec):
    s = books[sid]
    bids, asks = s["bids"], s["asks"]
    unit = (max((x["qty"] for x in bids + asks), default=1) / 8) or 1
    tag = f"{GRN}● live{RST}" if (bids and asks) else f"{DIM}○ …{RST}"
    lines = [f"{CYN}{name:<6}{RST} {tag}"]
    for lv in reversed(asks[:LEVELS]):
        lines.append(f"{RED}{fmt_px(lv['px'], dec):>13}{RST} {DIM}{bar(lv['qty'], unit)}{RST}")
    if bids and asks:
        mid = (bids[0]["px"] + asks[0]["px"]) / 2
        sp = (asks[0]["px"] - bids[0]["px"]) / mid * 1e4 if mid else 0
        lines.append(f"{WHT}{fmt_px(int(mid), dec):>13}{RST} {DIM}· {sp:.0f}bps{RST}")
    else:
        lines.append(f"{DIM}{'—':>13}{RST}")
    for lv in bids[:LEVELS]:
        lines.append(f"{GRN}{fmt_px(lv['px'], dec):>13}{RST} {DIM}{bar(lv['qty'], unit)}{RST}")
    while len(lines) < 2 + 2 * LEVELS:
        lines.append("")
    return lines


def vlen(s):
    out, i = 0, 0
    while i < len(s):
        if s[i] == "\x1b":
            while i < len(s) and s[i] != "m":
                i += 1
            i += 1
        else:
            out, i = out + 1, i + 1
    return out


def pad(s, w):
    return s + " " * max(0, w - vlen(s))


def render():
    cols = [book_lines(sid, name, dec) for sid, name, dec in TRIO]
    rows = max(len(c) for c in cols)
    out = [CLR, f"  {CYN}RSX{RST} {DIM}·{RST} {WHT}3 tokens trading live{RST} {DIM}· make demo · {time.strftime('%H:%M:%S')}{RST}", ""]
    for r in range(rows):
        line = "  "
        for c in cols:
            line += pad(c[r] if r < len(c) else "", COLW)
        out.append(line)
    out.append("")
    out.append(f"  {DIM}gateway :8088 · marketdata :8180 · maker quoting all three{RST}")
    sys.stdout.write("\n".join(out) + "\n")
    sys.stdout.flush()


async def main():
    ids = [str(sid) for sid, _, _ in TRIO]
    proc = await asyncio.create_subprocess_exec(
        CLI, "md", "follow", *ids, "-u", MD, "--json",
        stdout=asyncio.subprocess.PIPE,
    )

    async def rx():
        async for line in proc.stdout:
            apply_frame(json.loads(line))

    rxt = asyncio.create_task(rx())
    try:
        await asyncio.sleep(0.6)  # let the initial snapshots land
        for _ in range(FRAMES):
            render()
            await asyncio.sleep(0.7)
        await asyncio.sleep(1.2)
    finally:
        rxt.cancel()
        proc.terminate()
        await proc.wait()


if __name__ == "__main__":
    asyncio.run(main())
