# Round 2 Report — consumer wiring

Master at start: `9bdce56`. Master at end: `d009266`.

## Commits (7)

```
b519ac9 [cast] drop ReplicationConsumer::from_single; inline vec![addr]
f8d1c13 [cast] drop dead CastConfig::nak_retry_us field + env var
60faeb7 [cast] drop vestigial _stream_id arg from CastReceiver::new/with_config
dc2a6a6 [cast] drop CastReceiver::tick no-op + remove all callsites
54f0a56 [cast] demote is_faulted/is_reconnect_pending to pub(crate)
1415b7e [mark] use send_framed (one CRC) instead of paired prepare+send_raw
d009266 [matching] process_cancel uses publish_events; drop dead send_event helpers
```

## Round 1 hand-offs — all done

1. `ReplicationConsumer::from_single` dropped; 6 callers inlined `vec![addr]`.
2. `CastConfig::nak_retry_us` dropped (dead); env-var read + 2 test struct-literal inits cleaned.
3. `CastReceiver::new(_, _, _stream_id)` vestige dropped; 23 call sites updated.
4. `CastReceiver::tick()` no-op dropped; 10 call sites cleaned. `CastSender::tick()` kept (still emits idle-stream heartbeats).
5. `is_faulted` / `is_reconnect_pending` demoted to `#[cfg(test)] pub(crate)`.

## Consumer-side wins

- **Mark double-CRC** (1415b7e): `prepare → append_framed → send_framed` instead of `prepare → append_framed → send_raw`. Two sites (aggregate + sweep).
- **Matching cancel-path triple-CRC** (d009266): `process_cancel` rewritten to use `publish_events`. Was doing `write_events_to_wal + send_event_cmp loop + send_event_marketdata loop` — each event CRC'd three times. Now one CRC per event via the same fan-out the order-handling path uses. -207 LOC.

## Not actionable (verified, deliberately left)

- Gateway `next_seq()` / `advance_seq()`: gateway has no WalWriter, not a paired-CRC site.
- Risk `next_seq()` / `send_raw`: pure forwards (received-from-ME → sent-to-GW) or CMP-only emits. No WAL.
- `bench_match_rt.rs` paired prepare/send: different records (accepted is WAL-only; fill is built fresh). Not paired-CRC.

## Drive-by cleanups

- `cast_one_way_bench.rs` + `cast_rtt_bench.rs` headers: stale claims about receiver tick driving flow-control round-trips removed.
- `cast.rs:386` comment: `nak_retry_us` → `nak_debounce_us`.

## Per-crate LOC delta

```
rsx-cast               +29 -88   (-59)
rsx-matching            +6 -211  (-205)
rsx-mark               +14 -24   (-10)
rsx-risk                +4 -12   (-8)
rsx-marketdata          +4 -5    (-1)
rsx-recorder            +2 -2     (0)
rsx-cli                 +2 -5    (-3)
rsx-gateway             +1 -2    (-1)
TOTAL                  +62 -349  (-287 LOC)
```

## Tests

Baseline: 877 / 1 (pre-existing flaky `rsx_log::tests::drop_counter_increments_on_full_ring`).
Round 2 end: 877 / 1. Within plan budget.

## Round 3 hand-offs (spec + ARCHITECTURE sync)

1. `specs/2/4-cast.md` lines 230, 319, 602 still document `nak_retry_us`. Drop or replace with `nak_debounce_us`. Mirror in `rsx-cast/specs/4-cast.md` lines 339, 619.
2. `CHANGELOG.md` line 215 mentions `nak_retry_us`. Add v0.5.x entry noting the field removal + `_stream_id` / `from_single` / `tick()` API trims.
3. `rsx-matching/ARCHITECTURE.md` lines 19 + 42 still cite `write_events_to_wal` in cancel-path narrative. After Round 2, only replay + tests + benches call it; main.rs uses `publish_events` uniformly. Update.
4. `specs/2/17-matching.md` line 48: replace "Persist events — `write_events_to_wal`" with `publish_events` single-CRC description.
5. `specs/2/6-consistency.md` line 183 references `write_events_to_wal` sequencing. Re-verify invariant under `publish_events` (holds — `fan_out` appends WAL before each CMP send per-record).
6. `docs/benches.md` line 83 lists `write_events_to_wal` in `process_order_bench` — accurate for the bench harness; flag for audit but no change.
