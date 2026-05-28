# .ship/28 Results

Sprint: Refine + Audit Round 2  
Closed: 2026-05-28  
Master: `ee0c180`

## Rounds completed

| Round | Focus | Commit(s) |
|-------|-------|-----------|
| R1 | FAULTED replay wiring | 3b0facb, a5dbd72, 2ce0eb7 |
| R2 | Playground honesty F-N4–F-N16 | ea0deef |
| R3 | CTO carry-over (heap claim, BBO Vec, double-CRC) | 2f9ee7d |
| R4 | Bench infra + host tuning | a25f261, e997598 |
| Fix-up | F-N26/27/28/29/30 + zero-alloc margin | ee0c180 |

## Audit grades

| Audit | Grade | Notes |
|-------|-------|-------|
| CTO (pre-fix) | 68/100 hold | zero-malloc claim false, FAULTED log-only |
| CEO (pre-fix) | 51/100 | WAL_HDR V0/V1 mismatch broke 3 flows |
| CTO (post-fix) | ~78/100 | iterator fix makes hot-path claim true |
| CEO (post-fix) | ~68/100 | WAL parsing fixed; recorder wired; F-N31 sleep still open |

## Fixes shipped (fix-up commit ee0c180)

### F-N26, F-N27, F-N28 (MAJOR) — WAL_HDR V0/V1 mismatch
`server.py` WAL_HDR struct used V0 layout (record_type at offset 0).
Rust has written V1 (version at offset 0) since commit 64dda88.
Fix: `struct.Struct('<BBHHHi4s')` with V1 layout + reject non-V1 records.
Fixes: `/x/cmp-flows` showing 0 for all counters, `/x/book` KeyError,
`/api/verify` KeyError on bid_px.

### F-N29 (MODERATE) — Recorder connecting to UDP port
`RSX_RECORDER_PRODUCER_ADDR` pointed at `127.0.0.1:9110` (ME UDP casting
port). Recorder needs a TCP replication port.
Fix: added `BASE_ME_REPLICATION=9700` in start script; wired
`RSX_ME_REPLICATION_BIND_ADDR` on each ME instance; recorder now
connects to `127.0.0.1:{9700+sid}`.

### F-N30 (MODERATE) — Stale .pyc cache
Deleted `rsx-playground/__pycache__/` so Python regenerates on next import.

### CTO finding — positions_for_user allocates Vec on hot path
Replaced `positions_for_user() -> Vec<&Position>` with
`iter_positions_for_user() -> impl Iterator<Item = &Position>` —
zero-alloc flat_map over the positions_by_user index.
Changed `margin.calculate` and `margin.check_order` signatures to
`impl Iterator<Item = &'a Position>`. Updated tests + benches.
README "zero malloc on order hot path" claim is now accurate.

## Still open

- **F-N31**: gateway/marketdata `sleep(100µs)` busy-loop (monoio); tracked
  in `.ship/19-SLEEP-AUDIT`. Fix requires yield_now pattern change.
- **FAULTED apply closures**: wired to log-only; state re-injection is
  follow-up work (`.ship/25-CMP-RELIABILITY-V2`).
- **Task #6**: Re-run + reassemble all benches after rename.

## Test status

883 pass / 0 fail / 46 ignored (workspace).
