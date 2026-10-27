#!/usr/bin/env python3
"""Trace one order across the exchange's processes, for the demo GIF.

Submits a single IOC order through `rsx-cli order place` (the same gateway
WebSocket a real client uses) and prints the journey using the server's OWN
per-hop timestamps: each hop stamps the fill record as it travels
GW -> Risk -> ME -> Risk -> GW (specs/2/59-latency-observability.md).
Nothing here is synthesized -- the deltas are the ns stamps the exchange
put on the wire, shown as-is.

Run in the playground venv against a live cluster (`make demo`), from the
repo root:
    PYTHONPATH=rsx-playground rsx-playground/.venv/bin/python3 demo/trace_order.py
"""
import asyncio
import os
import time

import order_cli
import rsx_cli
import symbols

GW = os.environ.get("GATEWAY_URL", "ws://127.0.0.1:8088")
MD = os.environ.get("MARKETDATA_WS", "ws://127.0.0.1:8180")
DATABASE_URL = os.environ.get(
    "DATABASE_URL", "postgres://rsx:rsx@127.0.0.1:5432/rsx")
SYMBOL_ID = 10          # PENGU
USER_ID = 1

os.environ.setdefault(
    "RSX_GW_JWT_SECRET", "rsx-dev-secret-not-for-prod-padpad")

DIM = "\x1b[38;5;244m"
GRN = "\x1b[38;5;42m"
CYN = "\x1b[38;5;80m"
WHT = "\x1b[38;5;252m"
RST = "\x1b[0m"


async def best_ask() -> int:
    snapshot = await rsx_cli.run(
        "md", "snapshot", str(SYMBOL_ID), "-u", MD, "--json")
    bbo = snapshot["bbo"]
    if not bbo or not bbo["ask_px"]:
        raise SystemExit("no asks on the book -- is the maker quoting?")
    return int(bbo["ask_px"])


def say(line: str = "", pause: float = 0.35) -> None:
    print(line, flush=True)
    time.sleep(pause)


def ns_h(delta_ns: int) -> str:
    if delta_ns >= 1_000_000:
        return f"+{delta_ns / 1_000_000:7.1f} ms"
    return f"+{delta_ns / 1000:7.1f} µs"


def leg(label: str, delta_ns: int | None, note: str) -> None:
    shown = ns_h(delta_ns) if delta_ns is not None else f"{'—':>10}"
    say(f"   {WHT}{label:<14}{RST}{GRN}{shown}{RST}  {DIM}{note}{RST}")


async def trace() -> None:
    symbol = (await symbols.read(DATABASE_URL))[SYMBOL_ID]

    def px_h(px: int) -> str:
        return f"{px / 10**symbol.price_dec:.{symbol.price_dec}f}"

    def qty_h(qty: int) -> str:
        return f"{qty / 10**symbol.qty_dec:,.1f}"

    ask = await best_ask()
    # limit a hair above the touch: the maker re-quotes constantly, so a
    # limit exactly at a stale ask can miss; the fill still happens at the
    # book's real price, the limit is only the cap.
    limit = ask + max(10, ask // 200)
    outcome = await order_cli.Gateway(GW, DATABASE_URL).place(
        order_cli.NewOrder(
            user_id=USER_ID,
            symbol_id=SYMBOL_ID,
            side="buy",
            tif="ioc",
            px=limit,
            is_min_size=True,
        )
    )
    if outcome.status == "rejected":
        raise SystemExit(f"gateway rejected the order: {outcome.reason}")

    say(f" {CYN}RSX{RST} {DIM}·{RST} {WHT}one order, hop by hop{RST} "
        f"{DIM}· the exchange's own per-hop timestamps{RST}")
    say()
    say(f" {DIM}${RST} BUY {symbol.name} at the minimum size, up to "
        f"{px_h(limit)}  IOC   "
        f"{DIM}(user {USER_ID}, crossing the ~{px_h(ask)} ask){RST}", 0.8)
    say()
    say(f" {GRN}→{RST} N  new order over WS      {DIM}rsx-cli → "
        f"gateway :8088{RST}", 0.2)

    fills: list[list] = []
    for frame in outcome.frames:
        if "F" in frame and len(frame["F"]) >= 11:
            f = frame["F"]
            fills.append(f)
            say(f" {GRN}←{RST} F  fill px {px_h(int(f[2]))} "
                f"qty {qty_h(int(f[3]))}   "
                f"{DIM}maker oid {f[1][:8]}…{RST}", 0.2)
        elif "U" in frame:
            u = frame["U"]
            status = {0: "done", 1: "resting", 2: "cancelled",
                      3: "failed"}.get(int(u[1]), "?")
            say(f" {GRN}←{RST} U  {status}  filled {qty_h(int(u[2]))}  "
                f"remaining {qty_h(int(u[3]))}", 0.2)

    if not fills:
        raise SystemExit(
            "no fill came back -- stale cluster? reboot with make demo")

    # All legs from the FIRST fill's stamps: one physical record that
    # traveled GW -> Risk -> ME -> Risk -> GW, each hop stamping its own
    # field (mixing frames would mix emission times). Stamp index 7
    # (risk_in) is skipped -- unset today, see the closing caption.
    gw_in, me_in, match_done, gw_out = (
        int(fills[0][i]) for i in (6, 8, 9, 10))

    def since(stamp: int) -> int | None:
        # ns from gateway-in; — if the stamp predates it (skew/unset).
        return stamp - gw_in if stamp >= gw_in else None

    say()
    say(f" {WHT}the fill record's server-side stamps "
        f"(each hop wrote its own):{RST}", 0.6)
    say()
    leg("gateway in", 0,
        "WS parse -> cast/UDP -> Risk[user 1] margin gate")
    leg("matching in", since(me_in),
        "Risk -> cast/UDP -> ME[PENGU]  (leg includes the risk hop)")
    leg("match done", since(match_done),
        "price-time match in the engine")
    leg("gateway out", since(gw_out),
        "fill cast back ME -> Risk -> GW -> this WS")
    say()
    engine_ns = match_done - me_in if match_done >= me_in else 0
    say(f" {CYN}engine match {engine_ns} ns{RST} {DIM}·{RST} "
        f"{GRN}GW→ME→GW internal "
        f"{ns_h(gw_out - gw_in).lstrip('+ ')}{RST}", 0.4)
    say(f" {DIM}single-box dev cluster; risk_in not stamped yet; "
        f"Criterion benches own the perf claims{RST}", 0.4)


if __name__ == "__main__":
    asyncio.run(trace())
