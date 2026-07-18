#!/usr/bin/env python3
"""Trace one order across the exchange's processes, for the demo GIF.

Submits a single IOC order over the gateway WebSocket (the same wire a
real client uses) and prints the journey using the server's OWN per-hop
timestamps: each hop stamps the fill record as it travels
GW -> Risk -> ME -> Risk -> GW (specs/2/59-latency-observability.md).
Nothing here is synthesized -- the deltas are the ns stamps the exchange
put on the wire, shown as-is.

Run in the playground venv against a live cluster (`make demo`):
    ../rsx-playground/.venv/bin/python3 trace_order.py
"""
import asyncio
import json
import os
import time
import urllib.request
import uuid

import jwt as pyjwt
import websockets

GW = "ws://127.0.0.1:8088"
API = "http://127.0.0.1:49171"
SYMBOL_ID = 10          # PENGU
PRICE_DEC = 6
QTY_DEC = 4
LOT = 100_000
USER_ID = 1

DIM = "\x1b[38;5;244m"
GRN = "\x1b[38;5;42m"
CYN = "\x1b[38;5;80m"
WHT = "\x1b[38;5;252m"
RST = "\x1b[0m"


def token() -> str:
    secret = os.environ.get(
        "RSX_GW_JWT_SECRET", "rsx-dev-secret-not-for-prod-padpad")
    payload = {
        "sub": f"demo:{USER_ID}",
        "user_id": USER_ID,
        "exp": int(time.time()) + 600,
        "aud": "rsx-gateway",
        "iss": "rsx-auth",
        "jti": uuid.uuid4().hex,
    }
    return pyjwt.encode(payload, secret, algorithm="HS256")


def best_ask() -> int:
    with urllib.request.urlopen(f"{API}/api/book/{SYMBOL_ID}", timeout=5) as r:
        book = json.load(r)
    asks = book.get("asks") or []
    if not asks:
        raise SystemExit("no asks on the book -- is the maker quoting?")
    return int(asks[0]["px"])


def px_h(px: int) -> str:
    return f"{px / 10**PRICE_DEC:.{PRICE_DEC}f}"


def qty_h(qty: int) -> str:
    return f"{qty / LOT / 10**QTY_DEC * 10**QTY_DEC:,.1f}"


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
    ask = best_ask()
    # limit a hair above the touch: the maker re-quotes constantly, so a
    # limit exactly at a stale ask can miss; the fill still happens at the
    # book's real price, the limit is only the cap.
    limit = ask + max(10, ask // 200)
    qty = 20 * LOT
    cid = f"trace-{uuid.uuid4().hex[:8]}".ljust(20, "0")

    say(f" {CYN}RSX{RST} {DIM}·{RST} {WHT}one order, hop by hop{RST} "
        f"{DIM}· the exchange's own per-hop timestamps{RST}")
    say()
    say(f" {DIM}${RST} BUY {qty_h(qty)} PENGU up to {px_h(limit)}  IOC   "
        f"{DIM}(user {USER_ID}, crossing the ~{px_h(ask)} ask){RST}", 0.8)
    say()

    fills: list[list] = []
    async with websockets.connect(
            GW, additional_headers={"Authorization": f"Bearer {token()}"}
    ) as ws:
        await ws.send(json.dumps(
            {"N": [SYMBOL_ID, 0, limit, qty, cid, 1]}))
        say(f" {GRN}→{RST} N  new order over WS      {DIM}client → "
            f"gateway :8088{RST}", 0.2)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            try:
                raw = await asyncio.wait_for(ws.recv(), timeout=1)
            except TimeoutError:
                continue
            msg = json.loads(raw)
            if "F" in msg and len(msg["F"]) >= 11:
                fills.append(msg["F"])
                f = msg["F"]
                say(f" {GRN}←{RST} F  fill px {px_h(int(f[2]))} "
                    f"qty {qty_h(int(f[3]))}   "
                    f"{DIM}maker oid {f[1][:8]}…{RST}", 0.2)
            elif "U" in msg:
                u = msg["U"]
                status = {0: "done", 1: "resting", 2: "cancelled",
                          3: "failed"}.get(int(u[1]), "?")
                say(f" {GRN}←{RST} U  {status}  filled {qty_h(int(u[2]))}  "
                    f"remaining {qty_h(int(u[3]))}", 0.2)
                if int(u[1]) in (0, 2, 3):
                    break
            elif "E" in msg:
                raise SystemExit(f"gateway rejected the order: {msg['E']}")

    if not fills:
        raise SystemExit(
            "no fill came back -- stale cluster? reboot with make demo")

    # All legs from the FIRST fill's stamps: one physical record that
    # traveled GW -> Risk -> ME -> Risk -> GW, each hop stamping its own
    # field (mixing frames would mix emission times).
    gw_in, _risk_in, me_in, match_done, gw_out = (
        int(v) for v in fills[0][6:11])

    say()
    say(f" {WHT}the fill record's server-side stamps "
        f"(each hop wrote its own):{RST}", 0.6)
    say()
    leg("gateway in", 0,
        "WS parse -> cast/UDP -> Risk[user 1] margin gate")
    leg("matching in",
        me_in - gw_in if me_in >= gw_in else None,
        "Risk -> cast/UDP -> ME[PENGU]  (leg includes the risk hop)")
    leg("match done",
        match_done - gw_in if match_done >= gw_in else None,
        "price-time match in the engine")
    leg("gateway out",
        gw_out - gw_in if gw_out >= gw_in else None,
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
