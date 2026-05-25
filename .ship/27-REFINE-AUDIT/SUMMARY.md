# SUMMARY — .ship/27-REFINE-AUDIT

## Headline

Four rounds of targeted refine on rsx-cast + the five consumer crates,
followed by adversarial CTO + CEO audits and a P0/P1 critique-fix pass.
End-state: **system trades end-to-end again**, 879 / 0 / 46
(pass / fail / ignored) on `cargo test --workspace`, **net +405 LOC
over the sprint** (+1 718 / −1 313 across 77 files), with the bulk of
the diff being the v0.5.1 CHANGELOG entry (+70) and the new flow-
control deletions in the spec corpus. Code itself net-shrank (-318
LOC in Rust source across R1 + R2 alone).

## Commit count + dates

**46 commits**, all on 2026-05-24. Sprint window: `4329f32` (start) →
`1591362` (end). Diary entry written at end-of-sprint.

## Per-phase outcomes

| Phase | Master end | Commits | Net effect |
|---|---|---:|---|
| R1 — rsx-cast core hygiene | `9bdce56` | 9 | Dead-code purge in `cast.rs`/`wal.rs`/`replication_*`/`records`: dropped `tick()` no-op, `_stream_id` arg, `nak_retry_us` field, `from_single`, `highest_seen`, `stream_id` + `wal_dir` accessors, `segment_file_path`. -88 / +29 in rsx-cast. |
| R2 — consumer wiring | `d009266` | 7 | `mark` switched from paired `prepare → append_framed → send_raw` to single-CRC `send_framed`; `matching::process_cancel` rewritten to use `publish_events` fan-out (one CRC for WAL + risk + mkt destinations); `is_faulted` / `is_reconnect_pending` demoted to `pub(crate)`. **-287 LOC net.** |
| R3 — specs + ARCH sync | `3154f00` | 9 | Spec corpus brought to v0.5.x: `nak_retry_us → nak_debounce_us` (default 100 µs → 50 ms), `DxsReplay → ReplicationService`, WAL header version byte 0 + CRC32C (not CRC32 / V0 retired), `WalWriter::append → prepare + append_framed`, `cmp_* → cast_*` bench renames, full StatusMessage + flow-control chapter deletion, event buffer 10 000 → 65 536 heap-boxed. 8 distinct stale-element families swept across the spec + ARCH corpus. CHANGELOG v0.5.1 entry (+70 lines). |
| R4 — docs hygiene | `26e5753` | 6 | Root-doc sweep: `FEATURES.md` / `PROGRESS.md` / `BLOG.md` / `specs/2/41-testing-matching.md` / `rsx-cast/ARCHITECTURE.md` brought into line with R3's spec state. 4 items flagged for CTO + CEO: WAL retention documented-but-not-implemented, perf-table v0.2.0-lineage numbers, test-count drift across 6 surfaces (878/883/887/887+), and stale `specs/2/50-wedge.md`. |
| CTO audit | `5cca081` (read-only) | 0 | **58 / 100 — hold.** 5 claims verified; 3 attack scenarios traced. Confirmed: bytes-are-the-bytes invariant, publish_events one-CRC, ME WAL replay wired, JtiTracker wired. Disputed: WAL retention enforcement missing, README WalWriter::new arity wrong, "poll" method doesn't exist, JtiTracker "not wired" README line stale, `oldest_missing_run` unbounded loop on spoofed heartbeat, "zero heap on hot path" false in 4/4 consumers. |
| CEO audit | (read-only) | 0 | **14 / 100 — not yet.** Demo flow broken end-to-end: matching engine accepted 0 orders in 16 min, WAL stayed at 0 bytes, marketdata WS silent, latency probe timed out 100 %. Playground papered over it via timeout-as-accept (`server.py:4424` → green "accepted" on 2 s timeout). `mark` panicked on missing rustls `CryptoProvider`. 16 NEW findings (F-N1–F-N16); F-N1/N2/N3 critical. |
| FIX | `1591362` | 8 | P0/P1 from both audits shipped: env-var rename propagated to `start` (root cause of F-N1 — `RSX_*_CMP_*` was renamed in code but `start` still wrote the old names; ME bound 9100, Risk sent to 9110, mismatch → WAL stayed empty); maker auto-start dropped (UDP rmem overrun → Risk FAULTED → panic with no recovery wired); `mark` installs aws-lc-rs CryptoProvider; playground 504 + amber instead of green-on-timeout; WAL 4 h retention now actually enforced on `rotate()` (RETENTION_NS + prune); `oldest_missing_run` clamped to REORDER_CAPACITY (LAN DoS fix); rsx-cast README corrected (WalWriter arity, try_recv_with name, JtiTracker wired). Test count 878 → 879 (one new test for retention pruning). |

## Headline wins

1. **Demo trades end-to-end again.** Order → gateway → risk → me →
   fill → ORDER_DONE → cast → marketdata. Verified by submitting an
   IOC against a pre-seeded GTC and reading the 6-record WAL: ACCEPT
   (maker) → INSERT → ACCEPT (taker) → FILL → ORDER_DONE × 2.
2. **-318 LOC net in Rust source across R1 + R2** without losing
   functionality. The `publish_events` fan-out collapsed
   `write_events_to_wal + send_event_cmp loop + send_event_marketdata
   loop` into a single 14-line `fan_out` helper — one CRC per record
   for the WAL-and-cast path, where the old path computed it three
   times.
3. **Two compounding F-N1 root causes diagnosed.** The env-var rename
   in `3da5d9d` broke the spawn plan in `start` (port mismatch); the
   UDP rmem overrun from the auto-maker triggered Risk FAULTED →
   panic (POC-grade gap: FAULTED recovery via DXS replay isn't wired
   on Risk/marketdata/gateway). Both fixed in `7b46152` and `56c7ec4`
   respectively.
4. **WAL retention 4h: documented in 5+ surfaces for months, now
   actually enforced.** `prune_old_segments(wal_dir, stream_id,
   RETENTION_NS)` is called from `rotate()` end. ~30 LOC plus a
   `rotation_prunes_segments_older_than_retention` test that
   backdates segment mtimes and asserts they vanish on next rotate.
5. **`oldest_missing_run` LAN-DoS closed.** A spoofed
   `CastHeartbeat { highest_seq: u64::MAX - 100 }` previously walked
   the receiver's NAK-loop 2^64 times. Clamped to `from +
   REORDER_CAPACITY` (2 048 slots), matching the in-flight gap
   window the receiver tolerates before transitioning to FAULTED.

## Deferred / left for next sprint

Carried verbatim from FIX-REPORT "Deferred items":

- **F-N4** — synthetic-book `/x/book` from maker config (cosmetic
  badge needed).
- **F-N5** — `/api/latency-probe-gw` truthiness bug (needs default-
  symbol picker).
- **F-N6** — stress run `submitted=0 → PASS` (harness pass criterion).
- **F-N7, F-N8** — `/verify` mixes archive + live; PASSes 0-byte WALs.
- **F-N9, F-N10** — `/x/order-trace` doesn't read `tif`; renders IOC
  as "resting".
- **F-N11** — gateway CMP rebind AddrInUse on restart (supervisor
  needs `wait_for_port_free` helper).
- **F-N12** — gateway max-conn-per-user cap blocks dashboard bursts
  (playground needs WS pool reuse OR cap-lift for `user_id=1` in
  dev).
- **F-N13** — `/api/processes` restart counter stuck at 0 (UI wiring
  bug).
- **F-N14, F-N15, F-N16** — walkthrough crate/test count, CDN
  Tailwind on `/docs`, pulse pill 5/10 mismatch.
- **CTO #6** — consumers use allocating `try_recv`; root `CLAUDE.md`
  still asserts "Zero heap on hot path" unqualified. Needs a
  consumer-wide audit + a CLAUDE.md edit that names the qualifier.
- **CTO #7** — `rsx-risk/src/shard.rs:843-846` allocates
  `Vec<u32>` per BBO (R-N5 leftover).
- **CTO #9** — risk / marketdata / gateway panic on
  `CastRecv::Faulted`. Acknowledged POC-grade in `README.md` "What's
  not done". Multi-week scope; the proper fix is wiring DXS replay
  through all three consumers.
- **CTO #10** — BBO record CRC'd twice (cmp + mkt destinations); spec
  headline "one CRC per record" needs the BBO carve-out spelled out.
  Probably document, not fix.
- **`make tune-host`** target that bumps `/proc/sys/net/core/rmem_max`
  to 8 MB so the maker doesn't have to stay opt-in for demos.

## Next-sprint recommendation

Three buckets, ordered by ROI (from FIX-REPORT "Recommendation for
next sprint"):

1. **Wire FAULTED recovery for risk + marketdata + gateway** (CTO #9
   + F-N1 root cause #2). This is the only thing standing between
   "demo trades a few orders" and "demo trades at maker-realistic
   rates". The current "panic on FAULTED" is the entire reason the
   maker had to be made opt-in. One developer × one week.
2. **Playground honesty pass** (F-N4 – F-N10 + F-N13 – F-N16).
   Cluster the small UI lies into one sprint so the dashboard stops
   over-claiming. One developer × three days.
3. **Sysctl + bench harness for UDP rmem on demo hosts**: ship a
   `make tune-host` target so the maker doesn't have to be opt-in
   for the demo. Half a day, easy gating commit before any external
   walkthrough.

**Why this order**: (1) is the load-bearing correctness fix that
unblocks (3). Without FAULTED recovery, even with bigger rmem,
Risk/marketdata/gateway are one packet drop away from a panic on
production-grade flow. (2) is independent and easy to parallelise
with (1) on a separate branch. (3) is the smallest item and is
properly a follow-up to (1); without (1) it just delays the
inevitable.
