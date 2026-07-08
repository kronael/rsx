# 41 — rsx-matching CTO review (release pass, Step 3)

Adversarial review of `rsx-matching` for the internal release pass. Top
items independently verified against the code (file:line below). Verdicts:
**fix-now**, **doc-as-tradeoff**, **defer-to-BUGS**. Per the bug-triage
protocol the correctness/code items are filed in `BUGS.md` (dated section
2026-07-08 "rsx-matching release CTO review"), not fixed here — the founder
prioritises.

## Bucket 1 — correctness / hot-path

1. **Dedup not reconstructed across a snapshot boundary → post-crash resend
   can double-execute.** `rebuild_order_index_from_book` (main.rs:98)
   restores the order index from the slab but **not** `dedup`;
   `replay_wal_after_snapshot` only replays `RECORD_ORDER_ACCEPTED` for
   `seq >= start_seq` (post-snapshot). Snapshot cadence ~10 s; dedup window
   300 s (`dedup.rs:6`). So any order accepted >~10 s before a crash loses
   dedup protection on restart → a legitimate client resend of that `cid`
   within 5 min is treated as new and can double-fill. Known in code
   (`ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD` TODOs at main.rs:97,313-318) but
   **absent from BUGS.md** (verified: 0 hits). Verdict: **defer-to-BUGS**
   (file it; the fix — persisted dedup snapshot or wider replay — needs
   design). Violates the exactly-one-completion / dedup invariant.
2. **`ME-NEXT-SEQ-REGRESSION`** — logic verified correct (prevents seq
   regression when nothing replays past the snapshot); only the BUGS.md
   citation is missing. Two orphaned code→BUGS citations in one file →
   sweep the repo for more. Verdict: **defer-to-BUGS** (traceability).
3. **`REASON_DUPLICATE` (=3) in main.rs:52 is disconnected from
   rsx-book's `FAIL_*` namespace (0-2, event.rs:23-25).** No shared enum
   prevents collision; a future `FAIL_*`=3 in rsx-book would silently alias
   "duplicate" in `OrderFailedRecord.reason`, misleading risk/gateway about
   an order's fate. Verdict: **fix-now** (unify into one enum).

## Bucket 2 — minimization / dead code

4. **`write_events_to_wal` (wal.rs:54) and `publish_events` (wal.rs:240)
   are ~200-line near-duplicates that have already diverged:**
   `write_events_to_wal` no-ops on `Event::BBO` (wal.rs:230) while
   `publish_events` persists BBO (wal.rs:412) per the SEQ-1 fix. Nothing
   keeps them in sync. Verdict: **fix-now** (also causes finding 6).
5. **`process_cancel` (main.rs:955) reimplements the drift-check inline
   instead of calling `rsx-book::cancel_order_checked` (book.rs:355),
   missing that function's capacity-bound check.** Second place that must
   independently track `OrderSlot`'s layout. Verdict: **fix-now** (trivial).

## Bucket 3 — bench quality

6. **The flagship "266 ns full accept" bench silently omits BBO WAL writes**
   via the stale `write_events_to_wal` duplicate (finding 4). The cast-send
   omission is disclosed (`process_order_bench.rs:5`); the BBO-WAL omission
   is not, and BBO fires on most top-of-book-touching orders — so the
   headline understates real per-order cost. Verdict: **fix-now** (fold into
   the Step-4 re-bench).
7. **`match_by_depth` shares one live mutating book across the Criterion run**
   (unlike sibling `iter_batched` benches) — correct for proving
   depth-independence, undocumented divergence. Verdict: **doc-as-tradeoff**
   (add a comment).
8. **dedup benches never call `maybe_cleanup()`** so the map grows unbounded,
   unlike production's 5-min window — pessimistic, not misleading. Verdict:
   **doc-as-tradeoff**.

## Bucket 4 — doc gaps (fold into Step-1 docs-align)

9. **ARCHITECTURE.md:182-188 overclaims dedup is "WAL-persisted, not
   memory-only" across restarts** — true only post-snapshot (tracks finding
   1). Verdict: **fix-now** (soften to state the snapshot gap).
10. **README test inventory (README.md:84) lists 10 files, omits
    `replay_after_snapshot_test.rs` + `replay_fifo_test.rs`** (12 exist) —
    the crash-recovery tests most relevant to finding 1. Verdict:
    **fix-now**.
11. **ARCHITECTURE.md `main.rs:NNN` citations stale by 150-350 lines**
    (core-pin cited 195-200, actual 217-225; busy-loop 403 vs 524;
    config-poll 559 vs 808). Verdict: **fix-now** (mechanical).
12. **Measured-Performance table cites `reports/20260703_matching-benches.md`,
    which predates the fixture/FOK fixes (FIXED 2026-07-04).** Needs the
    Step-4 re-bench + a superseding report. Verdict: **doc-as-tradeoff**.

## Do-not-flag (confirmed out of scope)

RECENTER-EAGER-TAIL-SPIKE (already a documented tradeoff), casting
unauthenticated by design, ME not validating user input (gateway/risk own
it). No benches require Docker.

## Suggested sequencing

- **Now (triage):** file findings 1, 2 in BUGS.md (done in this pass).
- **Release fix-now (on founder go):** 3, 4→6 together (dedup the two WAL
  writers, which fixes the bench honesty), 5, plus doc findings 9, 10, 11.
- **Step 4 re-bench** supersedes finding 12 with a fresh report.
- **Design item (not a coding-agent task):** finding 1's real fix.
