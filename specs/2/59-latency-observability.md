---
status: planned
---

# 59 — Latency Observability (per-hop timestamps + Prometheus)

## Problem

The system needs to answer two different latency questions, and today it
answers neither fully on the live path:

1. **Per-event** — "how long did *this* order take, and where did the time
   go?" The terminal (`55-terminal.md`) shows a net/internal/engine split,
   but only `net` is real live (client-measured); internal/engine are
   placeholders because the server never reports them (bugs
   `GW-STAMP-LATENCY-LIVE`, `ME-ENGINE-LATENCY-NOT-REPORTED`).
2. **Aggregate** — "what's the p50/p99 of each stage over the last hour?"
   That is a metrics question, not a per-message one.

The naive fix — a bespoke `Latency` sidecar frame carrying pre-computed
deltas — was the *no-clock-sync* bootstrap. A production exchange runs PTP
(IEEE 1588, sub-µs; MiFID II mandates clock sync for timestamping), so the
richer, standard design is available: **per-hop timestamps in the records**.

## Design

### Per-event: per-hop timestamps embedded in the normal records, always

Every hop stamps its ingress (and, where meaningful, egress) time into the
record that flows back, extending the pattern already present (`ts_ns` on
every record; `taker_ts_ns` = gateway-ingress on `FillRecord`,
`rsx-messages`). The order lifecycle `GW → Risk → ME → Risk → GW` stamps:

- `gw_in_ns` — gateway ingress (exists as `taker_ts_ns`, generalise).
- `risk_in_ns` — risk tile received it.
- `me_in_ns` — matching received it.
- `match_done_ns` — match completed (ME).
- `gw_out_ns` — gateway egress to the client.

These land in the returned `FillRecord` / `OrderUpdate`-class records, in the
existing `repr(C, align(64))` **padding** (additive, record size unchanged).
Any consumer — terminal, recorder, replay — derives whatever delta it wants:
`engine = match_done − me_in`, `internal = gw_out − gw_in`, etc. The timing
is then also durable in the WAL, forever, for post-hoc analysis and audit.

**Clock model.** A duration measured within one host (one monotonic clock)
is always valid. A subtraction *across* hops assumes the hosts are
clock-synced — true under PTP in production, and trivially true on the
single-box dev/demo setup. This assumption is stated, not hidden: without
sync, only the within-host deltas (`engine`, `gw_in→gw_out` at the gateway)
are meaningful; cross-hop deltas need the sync the deployment provides.

**The client `net` leg is the one exception** — a trader's machine is never
PTP-synced to the exchange, so `net` (client↔gateway RTT) is always
client-measured (submit→ack on the client's clock). It is *not* a server
timestamp. This is the only field the webproto `Latency` message
(`49-webproto.md`) still needs to carry (client-filled); the internal/engine
legs move to record timestamps and the sidecar shrinks to the net leg.

### Aggregate: Prometheus via the existing pipeline (no hand-rolled message)

Long-term latency analytics are **metrics, not messages**. They ride the
telemetry path already specced in `33-telemetry.md`: hot-path code emits
structured log lines (the existing `latency_sample!`), **Vector** extracts
them into histograms, **Prometheus** scrapes Vector (`:9598/metrics`). No
Prometheus client library on the hot path; no bespoke analytics frame.
Latency histograms (per stage, per symbol) are added to the Vector transform
in `33-telemetry.md` — that is where "users read Prometheus off the systems"
is satisfied, reusing infrastructure instead of hand-rolling.

`rsx-health`'s `/metrics` (JSON snapshot, for HPA/load) is unchanged and
separate — it is liveness/load, not latency analytics.

## Plan (increments)

1. **`rsx-messages`** — add the per-hop `*_ns` fields to the returned
   records in existing padding; generalise `taker_ts_ns` → `gw_in_ns`.
   Wire size unchanged. Update record tests + `18-messages.md`.
2. **Stamps** — gateway (`gw_in_ns`/`gw_out_ns`), risk (`risk_in_ns`),
   matching (`me_in_ns`/`match_done_ns`, reusing the `latency-trace`
   per-stage measurement). Each stamps only its own hop.
3. **Terminal** — `rsx-term` reads the per-hop timestamps off the
   `Fill`/`U` records and computes internal/engine, replacing the
   `·· pending` placeholders with real live values; `net` stays
   client-measured. Retire the internal/engine placeholder path.
4. **Aggregate** — latency histograms in the `33-telemetry.md` Vector
   transform; document the Prometheus queries for stage p50/p99.

## References

- `22-perf-verification.md` — how latency is measured/gated (benches).
- `33-telemetry.md` — the structured-log → Vector → Prometheus pipeline.
- `49-webproto.md` — the `Latency` message (shrinks to the net leg).
- `55-terminal.md` — the terminal's speed/telemetry views (the consumer).
- `15-mark.md` — mark `max_source_lag_ns` freshness (the sibling
  "how stale is this number" case).
