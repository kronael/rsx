# 53 — WAL/Archive Read Service

## Problem

Non-Rust consumers re-implement the wire format. `rsx-playground/server.py`
carries a shadow copy of `rsx-messages`: `WAL_HDR = struct.Struct('<BBHHHi4s')`
plus `BBO_FMT`, `FILL_FMT`, `LIQN_FMT`, `OACC_FMT`, `OINS_FMT`, `ODONE_FMT`,
`MARK_FMT`, and the full `RECORD_*` constant table (server.py:1579-1621).
Every layout change in `rsx-messages` silently breaks these — it already has
(the WAL_HDR V0→V1 offset move broke `/x/book`, `/api/verify`, `/x/cmp-flows`
until server.py was hand-patched). This is the exact drift the Rust TUI avoids
by depending on `rsx-messages` directly.

## Principle

**One owner of the wire format.** Rust (`rsx-messages` + `rsx-cast::WalReader`)
is the only code that decodes WAL/archive bytes. Every other consumer —
playground, a future TS web UI, the TUI's history view — reads **decoded** data
(JSON) over a read API. No struct layout lives outside Rust.

This is the same "every concern has one owner" rule casting/matching follow
(see CLAUDE.md "Trust boundaries").

## Design

A small **read-only** Rust service that opens WAL streams + the archive
read-only, decodes with `rsx-messages`, and serves JSON over HTTP.

- **Decoder:** `rsx-cast::wal::WalReader` (`open_from_seq`, `next`,
  `read_record_at_seq`) + `rsx-messages` record types. Already used by
  `rsx-cli`.
- **Server:** hand-rolled HTTP/1.1 + JSON, zero async runtime — reuse the
  `rsx-health::spawn_health_server` pattern (std thread, no deps). Read-only
  GETs only. `serde_json` for bodies.
- **Trust:** internal, L3-trusted like casting (§10.4) — no auth on the
  service itself; the playground/gateway front it. Bind localhost by default.
- **Off critical path:** opens WAL files read-only; never writes; zero impact
  on the exchange hot path. Safe to run/kill freely.

### Placement — RECOMMENDED: new `rsx-read` crate

A dedicated single-concern crate `rsx-read` (bin), depending on `rsx-cast` +
`rsx-messages` + `rsx-health` (for the server primitive). Rationale:
- Single responsibility (read → JSON), no coupling to write paths.
- `rsx-cli` stays the interactive dump/inspect CLI (different UX, one-shot).
- `rsx-recorder` stays the archival *writer*; adding a read API couples read
  and write on the same binary.

**Alternatives (decide with codex):**
- **`rsx-cli serve`** — add a `serve --addr` subcommand; reuses rsx-cli's
  existing record-decode helpers verbatim, one fewer crate. Cost: turns a CLI
  into a hybrid CLI+server (mild smell).
- **extend `rsx-recorder`** — it already holds the archive on disk. Cost:
  couples the archival writer with a query API; the recorder is on the
  replication path.

Whichever wins, factor the record→JSON decode into a shared helper so
`rsx-cli` and the service never diverge.

## API (derive from the playground's actual WAL reads)

Minimal, read-only. Names illustrative; finalize during impl.

- `GET /streams` → `[{name, tip_seq, first_seq, byte_size}]` (replaces
  `scan_wal_streams`).
- `GET /streams/{name}/records?from=SEQ&limit=N&types=FILL,BBO` → decoded
  records as JSON (replaces the per-format `struct.unpack` loops + the
  timeline).
- `GET /streams/{name}/counts?types=&from=SEQ` → `{FILL: n, BBO: n, …}`
  (replaces `_cast_pipe_counts`' WAL-side counting; note gw/risk write no WAL —
  see CLUSTER-HEALTH-ADDR-UNSET, those come from /metrics not here).
- `GET /book/{symbol}?depth=N` → L2 snapshot reconstructed from the ME WAL
  (replaces `/x/book`'s Python reconstruction). Reuses `rsx-book`/marketdata
  shadow logic if cheap, else a straight fold over records.
- `GET /health` `/ready` — standard `rsx-health` endpoints.

Consistency checks (`/api/verify`) stay in the playground as *logic*; they
consume `/streams/.../records` instead of parsing bytes.

## Consumers

- **playground:** migrate `scan_wal_streams`, `_cast_pipe_counts`, `/x/book`,
  `/api/verify`, WAL timeline to call the service. **Delete** the struct table
  (server.py:1579-1621) + the `RECORD_*` constants. Net-negative Python;
  removes the whole WAL_HDR-version bug class.
- **future TS web UI / TUI history view:** same API, no re-decode.

## Phasing

1. **CLI-shim first (unblock + prove):** playground shells out to `rsx-cli`
   (already decodes) for one reader (e.g. `/streams`) to validate the
   decoded-data-over-boundary pattern with ~zero new code.
2. **Service:** build `rsx-read` (or the chosen placement) with the endpoints
   above; migrate playground readers one at a time (the 421-test gate is the
   net); delete the Python struct table last.
3. **Verify:** each migrated endpoint matches the old Python output (diff a
   few fixtures) before deleting the Python decoder it replaces.

## Non-goals

- Not a write path; not on the critical path; no order submission.
- No auth on the service (internal, L3-trusted); front it at the playground.
- Not a general query engine — just the reads the playground/UI need.
- No external publishing.

## Open questions (for codex critique)

1. Placement: new `rsx-read` crate vs `rsx-cli serve` vs recorder-extension —
   which minimizes total complexity?
2. Is hand-rolled HTTP (rsx-health style) enough, or does the record-query
   surface justify a light framework? (Bias: hand-rolled, per zero-dep ethos.)
3. `/book/{symbol}` reconstruction — reuse marketdata's shadow book, or a
   standalone fold? Avoid a second book implementation.
4. Archive vs hot-tier reads: one endpoint that transparently falls through
   hot WAL → archive (like the replication NAK two-tier), or separate paths?
5. Fixture/parity testing: how to assert the service's JSON matches the
   retiring Python decoder before deletion.
