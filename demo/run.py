#!/usr/bin/env python3
"""Driven demo for the GIF: three live RSX order books, side by side, moving.

Consumes the AUTHORITATIVE marketdata WS (ws://127.0.0.1:8180) directly —
subscribe + app-level heartbeat + protobuf decode via the playground's
md_wire — so it shows the real exchange book (the /api/bbo dashboard path is a
separate, flaky reconstruction). The RSX system must be up (`make demo`).

Run in the playground venv (needs websockets + md_wire):
    ../rsx-playground/.venv/bin/python3 run.py
"""
import asyncio
import json
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "rsx-playground"))
import md_wire  # noqa: E402
import websockets  # noqa: E402

MD = "ws://127.0.0.1:8180"
CHANNELS = 7  # depth + BBO + trades
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
    if "B" in f:
        sid, bids, asks = int(f["B"][0]), f["B"][1], f["B"][2]
        if sid in books:
            books[sid] = {
                "bids": [{"px": b[0], "qty": b[1]} for b in bids],
                "asks": [{"px": a[0], "qty": a[1]} for a in asks],
            }
    elif "D" in f:
        arr = f["D"]
        sid, side, px, qty = int(arr[0]), int(arr[1]), int(arr[2]), int(arr[3])
        if sid not in books:
            return
        key = "bids" if side == 0 else "asks"
        lv = books[sid][key]
        if qty == 0:
            books[sid][key] = [x for x in lv if x["px"] != px]
        else:
            hit = next((x for x in lv if x["px"] == px), None)
            if hit:
                hit["qty"] = qty
            else:
                lv.append({"px": px, "qty": qty})
            lv.sort(key=lambda x: -x["px"] if side == 0 else x["px"])
    elif "BBO" in f:
        # authoritative touch: drop crossed levels, upsert the top
        arr = f["BBO"]
        sid, bpx, bqty, apx, aqty = (int(arr[0]), int(arr[1]), int(arr[2]),
                                     int(arr[4]), int(arr[5]))
        if sid not in books:
            return
        s = books[sid]
        if apx:
            s["bids"] = [x for x in s["bids"] if x["px"] < apx]
        if bpx:
            s["asks"] = [x for x in s["asks"] if x["px"] > bpx]
        for key, px, qty, desc in (("bids", bpx, bqty, True),
                                   ("asks", apx, aqty, False)):
            if not px:
                continue
            hit = next((x for x in s[key] if x["px"] == px), None)
            if hit:
                hit["qty"] = qty
            else:
                s[key].append({"px": px, "qty": qty})
            s[key].sort(key=lambda x: -x["px"] if desc else x["px"])
    elif "T" in f:
        sid = int(f["T"][0])
        if sid in books:
            last_trade[sid] = (int(f["T"][1]), int(f["T"][2]))


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
    async with websockets.connect(MD) as ws:
        for sid, _, _ in TRIO:
            await ws.send(json.dumps({"S": [sid, CHANNELS]}))

        async def hb():
            while True:
                await asyncio.sleep(4)
                await ws.send(json.dumps({"H": [int(time.time() * 1000)]}))

        async def rx():
            async for msg in ws:
                if isinstance(msg, (bytes, bytearray)):
                    fr = md_wire.decode(bytes(msg))
                    if fr:
                        apply_frame(fr)

        hbt = asyncio.create_task(hb())
        rxt = asyncio.create_task(rx())
        await asyncio.sleep(0.6)  # let the initial snapshots land
        for _ in range(FRAMES):
            render()
            await asyncio.sleep(0.7)
        await asyncio.sleep(1.2)
        hbt.cancel()
        rxt.cancel()


if __name__ == "__main__":
    asyncio.run(main())
