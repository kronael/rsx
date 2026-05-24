# RSX

Spec-first perpetuals exchange. 12 Rust crates, 25k LOC, 887 tests.

## Problem

Exchanges treat the trade-data plane as a private moat. The components
that matter — append-only log, replay protocol, fixed-format wire —
get reinvented every time, each version locked inside one venue's
code. Result: every team building risk, surveillance, market-data, or
a competing venue rebuilds the same pieces, badly.

## What we built

Two layers, one repo.

1. **rsx-dxs** — domain-agnostic transport: WAL + CMP (C-struct UDP)
   + TCP replay. Same bytes on disk, in UDP, on the wire. No
   knowledge of orders, fills, or users. Reusable wherever you need
   an audited stream of fixed-size records.

2. **The rest** — orderbook, matching, risk, gateway, marketdata,
   mark, recorder, maker. A working perp exchange on top of rsx-dxs.
   Separate processes, each pinned, each one tile per concern.

## Why it's different (the wedge)

B+A — see `specs/2/50-wedge.md`.

- **B (open-source orthogonal libs)**: rsx-dxs is the headline. WAL
  format = wire format = stream format is a meaningful design choice
  almost nobody makes; it earns you free correctness (one parser,
  one fuzz target) and free observability (one tool, three views).
- **A (exchange-in-a-box)**: the rest layered on top. Buyers who
  want a venue get one; buyers who want only the rails get rsx-dxs.

The boundary is enforced in the build: `cargo tree -p rsx-dxs
--edges normal` shows zero rsx-types/rsx-messages dependency.

## Proof

Three concentric circles of measurement (`.ship/18-COMPONENT-BENCHES/
LANDSCAPE.md`, `docs/benches.md`):

| Layer | p50 | What it is |
|---|---:|---|
| Match algorithm | **340 ns** | dedup + WAL accept + match + WAL events |
| In-process round-trip | **9.58 µs** | real CMP/UDP + Orderbook + WAL, one binary |
| Cross-process production | **1 128 µs** | GW→ME→GW, separate processes |
| Cross-process via Python WS probe | **11 878 µs** | end-to-end with client framing |

99% of the in-process round-trip is the `sendto` syscall body
(`rsx-dxs/benches/cmp_send_breakdown_bench.rs`). Algorithm + framing
add up to <0.7% of wall time. Optimisation targets are kernel-bypass
shaped, not Rust-shaped, and `specs/2/4-cmp.md` already documents the
DPDK / AF_XDP "Later" path. Honest.

## What's open

- Monoio-native UdpSocket on `CmpReceiver` (~50 LOC) — kills the
  100 µs sleep that explains ~655 µs of cross-process p50.
- Adversarial CTO + CEO audits round 2 (`.ship/20-CTO-CEO-REVIEW-2/`,
  in-flight at time of writing).
- PG write-behind tail (p99 = 233 ms; p50 unaffected).

No published binaries. No production users. No SLA promised.
GitHub-only; see `CLAUDE.md` "Publishing" section.
