# demo/ — the system-level demo area

Demos of the WHOLE exchange, as opposed to one surface: the terminal
demo (`../rsx-term/demo/`, the client's view) and the dashboard tour
(`../rsx-playground/demo/`, the operator's toolset) live with their
crates. This directory holds what cuts across them — the 3-token board
(`run.py`) and the whole-exchange walkthrough (`exchange.gif`). Inherits
the repo-root `../CLAUDE.md`.

## The whole-exchange walkthrough (`exchange-walkthrough.sh`)

The ARCHITECTURE demo: how an order flows across the separate processes,
told with live captures — browser shots of the dashboard's Topology/WAL
pages, terminal recordings, and caption cards (`cards/*.html`, Ayam
Cemani palette). Nothing is faked: real cluster, real fills, real WAL
records, and every latency figure is a server-side stamp shown as-is.

1. **Topology** — the process graph all-green on the trio (9/9: gateway,
   risk-0, me-pengu/me-sol/me-btc, marketdata, mark, recorder, maker),
   the Matching detail card with three symbols' books, the core-affinity
   map and live cast-connection counters.
2. **An order's journey** (`trace_order.py`) — one IOC buy over the
   public gateway WS; the timeline prints the fill record's own per-hop
   stamps (`gw_in → me_in → match_done → gw_out`,
   specs/2/59-latency-observability.md). Legs come from ONE frame's
   stamps — mixing F/U frames mixes emission times. `risk_in` is unset
   today (rsx-messages FillRecord doc); the GW→ME leg includes the risk
   hop and the caption says so.
3. **The tape** (`wal_peek.sh`) — the SAME oid read back from the ME's
   WAL via rsx-cli: ORDER_ACCEPTED → FILL(s) → ORDER_DONE, fills
   preceding DONE (invariant #1), plus the two-transports card
   (casting/UDP live, WAL/TCP cold).
4. **Scale-out** — `run.py`'s three live books beside the process list's
   three ME processes.

Honesty rails: the traced numbers are single-box dev-cluster values and
the closing card says perf claims live in the Criterion benches (root
CLAUDE.md "Latency & performance verification"). Casting is
unauthenticated BY DESIGN (specs/2/4-cast.md §10.4) — never present it
as a flaw.

## How it's rendered

`exchange-walkthrough.sh` boots a fresh trio (`reset --yes` + `make
demo`), records the terminal beats with asciinema → agg, shoots the
dashboard pages and cards with agent-browser (1280×800), then
`assemble_exchange_gif.py` (playground venv)
letterboxes everything onto 960×600 and writes per-frame durations;
gifsicle shrinks the result.

    make -C demo exchange-gif                # full: fresh trio + capture
    SKIP_PREP=1 ./demo/exchange-walkthrough.sh   # cluster already up + fresh

## Fresh-cluster runbook

- The order path degrades minutes after boot (BUGS.md
  `RISK-CMP-DESYNC-ORDER-EVENT-STARVE`), so the script records the order
  trace FIRST and the trace crosses a few ticks past the displayed ask —
  a limit at a stale touch misses the re-quoting maker. If the trace
  reports no fill or ms-scale legs, reboot and re-record; never present
  a degraded run as the demo.
- Frame checkers: extract the GIF frames (PIL) and confirm the trace
  filled (F lines + stamp table), the wal-peek shows the same oid
  lifecycle unwrapped, topology shows 9/9 running with three MEs, and
  the trio frames show three moving uncrossed books.

## Artifacts

- `exchange-walkthrough.sh` — the driver (checked in; reproducible).
- `trace_order.py`, `wal_peek.sh`, `assemble_exchange_gif.py`,
  `cards/*.html` — the beats it drives.
- `exchange.gif` — the rendered walkthrough (embedded in `../README.md`
  "How it scales").
- `run.py` + `render_cast.py` — the 3-token board and its fallback
  renderer (`orderbook-gif` target).
