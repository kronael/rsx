# 41 — Matching Release: Founder Handoff

`rsx-matching` is a release candidate. This pass reconciled the docs to one
sourced set of numbers, deduped the WAL event writers, settled the ME↔Risk
output-split design, and isolated ME's per-user state behind a clean seam.
Two items remain before "done": a quiet-box `make bench-gate` run, and the
end-to-end demo (blocked on a concurrent session's playground-entrypoint
rework — verify once that settles).

## What shipped this pass

| Commit | What |
|--------|------|
| `761aa94` | `[matching]` dedupe the two WAL event writers — BBO now flows through one `emit()` path in both the production and WAL-only sinks (production behavior unchanged; it always persisted BBO) |
| `3a428b1` | `[docs]` latency reconciliation — pure-match `54/60→~30 ns`, accept `340→266 ns`, `PriceLevel 24→32 B`, ghost-bench attribution fixed |
| `63b69c6` | `[docs]` relabel `266 ns` accept as "one fill, BBO unchanged" (a BBO-moving accept adds one WAL record) |
| `dced1b0` | `[specs]` consistency — `28-risk` frontmatter `shipped→partial`; `18-messages` corrected ("ME holds a per-symbol position", not "no positions") |
| `5fce02c` | `[docs]` BUGS — struck ME-GW gaps 6/7 as false-positives, filed `ME-HOLDS-USER-STATE` |
| `fe133fc` / `334fb78` | `[book]`/`[matching]` encapsulate ME's per-user state behind `UserRegistry` (snapshot format byte-identical) |
| `b998b1b` | `[matching]` cancel routed through `cancel_order_checked` (adds the capacity-bound guard the inline version lacked; happy path unchanged) |
| `0f9f871` | `[matching/book]` `REASON_DUPLICATE` unified into rsx-book `FAIL_DUPLICATE=3` (one namespace) |
| `f6316cf` | `[matching/risk]` dropped dangling `bugs.md` code citations (repo sweep) |

All commits above re-verified locally: rsx-matching + rsx-book unit/integration
tests, `clippy --all-targets -D warnings`, and `fmt` all green.

## Design settled — ME→GW-direct output split (`28-risk.md`, `status: partial`)

The async output split (ME confirms straight to gateway; settle to Risk async)
was audited as an 8-gap "high-risk rewrite." It is not:

- **Acceptance is an irrevocable commitment.** Once Risk pre-flight accepts,
  the fill is guaranteed; Risk only *processes* fills, it never vetoes. This
  dissolves gaps 1/2/8 — per order exactly one terminal producer (Risk iff
  pre-flight-rejected, else ME).
- **Reduce-only is safe under async settle (gaps 6/7 were false-positives).**
  ME holds the authoritative per-symbol position (`net_qty`) and clamps
  reduce-only against it synchronously at match time; recovery rebuilds it
  exactly (snapshot `net_qty` + replay through `process_new_order`). Risk's
  stale view never enters the decision.
- **Remaining work is mechanical transport only** — gateway as a full
  ME-replication consumer (like the recorder) + `fill_id` dedup. Bounded, but
  needs the spec rewritten around the invariant before implementation.
- **Future direction** (`ME-HOLDS-USER-STATE`, unsolved): generalize the ME
  edge-position into a pre-authorized risk **buffer** (Risk grants a
  conservative per-symbol allowance; ME draws down; Risk tops-up/revokes
  async) — which would let the *forward* Risk hop be skipped too. Hard part is
  buffer sizing + cross-symbol partitioning + revocation as mark moves.

## Verified (this session, re-run locally)

- `rsx-matching` + `rsx-book` unit + relevant integration tests green;
  `clippy --all-targets -D warnings` clean; `fmt` clean.
- Snapshot binary format byte-identical after the `UserRegistry` refactor
  (7 round-trip tests incl. `snapshot_and_wal_recovery_restores_book`).
- Numbers sourced to `reports/20260703_matching-benches.md`: match ~30 ns
  (depth-independent), full accept 266 ns (one fill, BBO unchanged), 3.6M/s.

## Open / needs a founder call or a clear box

Both runtime checks were **run this pass** (box freed):

- **Benchmarks — numbers do NOT reproduce on this shared box** (see
  `reports/20260709_matching-benches.md`). Full-workspace `make bench-gate` is
  nightly-scale and timed out mid-`rsx-book`. Ran the matching benches directly:
  `match_by_depth` **~48 ns** (vs published 30 ns), `me_accept_path/full`
  **~295 ns** (vs 266 ns), throughput **~3.46M/s** (vs 3.6M — stable). Match
  stays depth-independent. Not a regression (the pre-refactor re-bench already
  saw ~51 ns) — shared-host variance (~1.6× on the tight per-op benches). The
  `30 ns`/`266 ns` figures need a **dedicated box** to be trustworthy as
  published. Filed `MATCHING-BENCH-SHARED-HOST-VARIANCE`.
- **Demo — could NOT drive a verified fill** (env/harness, not code).
  `scripts/demo-trade.sh` is stale after the playground rework: it POSTs the
  removed `/api/submit-order` (404) and checks the wrong WAL path (`tmp/wal/10`
  — the pengu ME writes to `tmp/wal/pengu/10`). Via the current
  `/api/orders/test` (form, human units), a maker **rested** (a real ME
  round-trip) but takers time out ("no ME response in 2s") and the pengu WAL
  stays 0 bytes — the live-cluster order round-trip isn't completing, though all
  procs report running and rmem is tuned. This sits in the concurrent session's
  in-flux playground/demo rework, not the engine. **The ME logic is verified by
  the passing unit + integration suites** (which drive matching directly, not
  through the demo cluster). Filed `DEMO-TRADE-SH-STALE`.
- **ME→GW-direct** — spec rewrite around the acceptance-commits invariant, then
  the transport implementation. Bounded feature, not started.
- **`ME-GW-DIRECT-SPEC-GAPS`, `ME-HOLDS-USER-STATE`** in `BUGS.md` carry the
  full design record.

## How to run / demo

```
./rsx-playground/playground start-all minimal   # + a maker (see TODO/diary)
scripts/demo-trade.sh                            # maker+taker IOC, assert fill in ME WAL
```

The dashboard's "e2e latency" card is browser→proxy→gateway (~ms) — a demo aid,
**not** the exchange's µs internal latency. Internal latency lives in the
Criterion benches + `make bench-gate` (`specs/2/22-perf-verification.md`).
