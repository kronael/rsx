# Round 3 Report — specs + ARCHITECTURE sync to v0.5.x

Master at start: `b49de63`. Master at end: `3154f00`.

## Commits (9)

```
b36efd6 [spec] cast: nak_retry_us -> nak_debounce_us; drop dead config fields
3649b49 [spec] matching: publish_events one-CRC fan-out replaces write_events_to_wal
3a42e35 [changelog] rsx-cast v0.5.1: Round 1+2 API trims
5ebef73 [spec] DxsReplay -> ReplicationService across spec corpus
190b6a6 [spec] WAL header: version at byte 0; CRC32C (not CRC32); V0 retired
b09eb1a [spec] WalWriter::append -> prepare+append_framed; cmp_* bench renames
9aabe2f [spec] cast: drop StatusMessage + flow-control narrative from standalone copy
73b4503 [spec] purge StatusMessage / flow-control narrative from broader corpus
3154f00 [arch] event buffer [Event; 10_000] -> heap-boxed MAX_EVENTS=65_536
```

## Hand-offs from Round 2 — all done

1. **`specs/2/4-cast.md` + `rsx-cast/specs/4-cast.md`** — `nak_retry_us` renamed
   to `nak_debounce_us` at lines 230 / 319 / 602 (parent) and 339 / 619
   (standalone). Default updated 100 µs → 50 ms. Recovery budget arithmetic
   updated 800 µs → 400 ms. Drive-by: dropped `send_ring_limit` row from the
   CastConfig table (it was never a config field — it's the
   `SEND_RING_CAPACITY` compile-time constant in `cast.rs`).
2. **`CHANGELOG.md`** — new v0.5.1 section covers `WalWriter::append`
   removal, `ReplicationConsumer::from_single` drop, `CastReceiver::new`
   signature change, `tick()` no-op removal, `is_faulted` /
   `is_reconnect_pending` demotion, `nak_retry_us` field drop, accessor
   removals, and path-layout helper extraction in `wal.rs`.
3. **`rsx-matching/ARCHITECTURE.md`** — module table + main-loop listing
   updated. `publish_events()` is the primary; `write_events_to_wal()` is
   tagged as "replay + bench helper". Loop pseudocode rewritten to use
   `publish_events` for the order + cancel paths uniformly.
4. **`specs/2/17-matching.md`** §Order Acceptance Flow step 4 rewritten:
   `publish_events` semantics (one CRC, BBO seq path, best-effort cast
   sends, WAL-failure-panics).
5. **`specs/2/6-consistency.md`** invariant #1 prose updated: cites
   `publish_events` and documents the per-event `append_framed`-then-`send`
   ordering that preserves "Fills precede ORDER_DONE" (`fan_out` writes
   WAL per record before fanning the same `Framed` to cast destinations,
   so on-disk and on-wire order match buffer order).
6. **`docs/benches.md`** line 83 — kept the `write_events_to_wal`
   reference (accurate for the bench harness) and added a note that
   production uses `publish_events`. No regression risk.

## Wider audit results — terminology sweep counts

| Stale element | Files touched | Notes |
|---|---:|---|
| `DxsReplay` → `ReplicationService` | 8 | Across spec corpus (1-architecture, 10-replication, 15-mark, 21-orderbook, 36-testing-replication, 39-testing-mark, 41-testing-matching, rsx-cast/specs/10-replication). |
| `version: u8 at byte 8` → `byte 0`, drop `V0` | 8 | ARCHITECTURE.md (root + rsx-cast), specs/2/4-cast, 10-replication, 48-wal; rsx-cast/specs/4-cast, 10-replication, 48-wal. Layout diagrams + version-policy prose. |
| `CRC32` → `CRC32C (Castagnoli)` | 5 | rsx-cli/ARCHITECTURE, rsx-cast/ARCHITECTURE, ARCHITECTURE.md, specs/2/48-wal, docs/benches; existing CRC32C citations in /specs/2/4-cast were already current. |
| `WalWriter::append` → `prepare` + `append_framed` | 7 | Perf tables in ARCHITECTURE.md, specs/2/4-cast, 22-perf-verification, 36-testing-replication, 45-tiles, 48-wal, rsx-cast/specs/4-cast; also pseudocode in 15-mark and rsx-cast/ARCHITECTURE module table. |
| `StatusMessage` / `RECORD_STATUS_MESSAGE` / flow-control | 6 | rsx-cast/specs/4-cast (whole §5 dropped + ToC renumbered + §1 tagline + §3 control-message list + §7 Aeron-comparison line + §10 config note), specs/2/10-replication + rsx-cast/specs/10-replication record-type list, specs/2/18-messages (record table), specs/2/29-rpc §casting Flow Control, specs/2/35-testing-cast (drift notice), specs/2/51-cmp-v2-multicast (Flow control + Status messages sections + implementation order). |
| `cmp_*_bench` → `cast_*_bench` | 4 | docs/benches, specs/2/22-perf-verification, specs/2/35-testing-cast, specs/2/52-blog, rsx-cast/specs/4-cast. |
| `src/cmp.rs` / `src/client.rs` / `src/server.rs` paths | 6 | rsx-cast/specs/4-cast (line-ref grep sweep `cmp.rs:` → `cast.rs:` × 7 sites), specs/2/4-cast, specs/2/5-codepaths, specs/2/6-consistency, specs/2/10-replication + rsx-cast/specs/10-replication module-layout codeblocks. |
| `protocol.rs` → `records.rs` (rsx-cast module) | 4 | rsx-cast/ARCHITECTURE, ARCHITECTURE.md, specs/2/10-replication + rsx-cast/specs/10-replication, specs/2/18-messages. |
| `[Event; 10_000]` → `Box<[Event; MAX_EVENTS]>` (65_536) | 5 | ARCHITECTURE.md, rsx-book/ARCHITECTURE, rsx-matching/ARCHITECTURE, specs/2/21-orderbook (§6 + §7), specs/2/41-testing-matching M24. |

## Per-file edits (non-trivial)

- **`CHANGELOG.md`** — net +70 lines for the new v0.5.1 entry. No
  modifications to historical entries.
- **`rsx-cast/specs/4-cast.md`** — full §5 "Flow control" chapter
  (~25 lines) and the §4 "StatusMessage" subsection (~18 lines)
  removed; ToC re-listed (§6-11 → §5-10), §9.2 cross-ref renumbered
  (was §10.2), §5 cross-ref renumbered (was §6); tagline rewritten;
  "Aeron sequence-window flow control" Aeron-comparison line
  rewritten to "deliberately dropped vs Aeron".
- **`specs/2/1-architecture.md`** — crate map dropped stale `rsx-maker`
  row; added `rsx-messages` + `rsx-log` rows. Diagram updated
  `DxsReplay TCP server` → `Replication Service TCP`.
- **`specs/2/15-mark.md`** — pseudocode now shows the single-`Framed`
  fan-out pattern explicitly (`prepare` once → `append_framed` →
  `send_framed`), instead of the v1 `wal.append(...) + cmp.send(...)`
  pair.
- **`specs/2/35-testing-cast.md`** — top-of-file drift notice flagging
  StatusMessage rows as audit history; test file layout updated to
  match actual `src/<module>_test.rs` inline convention from `184c3c4`;
  integration row tests renamed `cmp_test.rs` → `cast_test.rs`,
  `client_test.rs` → `replication_client_test.rs`.
- **`specs/2/29-rpc.md`** — §casting Flow Control rewritten: dropped
  the StatusMessage/window pseudocode; documented removal in `87b223e`
  and that app-level rate limiting is now the only backpressure.
- **`specs/2/51-cmp-v2-multicast.md`** — future multicast spec
  rewritten to reflect v1's no-flow-control state: §Status messages
  bullet, §Flow control section, and the per-receiver-window step in
  the implementation order all changed.

## Drive-bys (not in Round 2 hand-offs)

- `specs/2/6-consistency.md` invariant #5:
  `rsx-cast/src/client.rs::DxsClient::run_*` →
  `replication_client.rs::ReplicationConsumer::run_*`.
- `specs/2/23-playground-dashboard.md`: `GET /cmp/flows` → actual
  endpoint name `GET /x/cmp-flows` (rsx-playground `server.py:3278`).
- `specs/2/6-consistency.md` ToC anchor:
  `#drain-loop-pseudocode-cmp` → `#drain-loop-pseudocode-casting`
  (matches the actual GitHub auto-anchor for the renamed section
  header).

## Cargo check + tests

- `cargo check --workspace` — finished in 0.14s, all green.
- `cargo test --workspace --lib --tests` — 372 pass total; only
  failure is the known-flaky
  `rsx_log::tests::drop_counter_increments_on_full_ring`
  (pre-existing per MEMORY.md, passes in isolation). No regressions
  attributable to spec/markdown changes.

## Round 4 hand-offs (docs hygiene + cross-cut consistency)

Now that specs are current, Round 4 should sweep the root-level
narrative + dashboards. Specific findings to action:

1. **README.md** likely still has the v0.4.x rsx-dxs narrative.
   Stale things to grep for: `rsx-dxs`, `DxsConsumer`, `cmp_`
   bench filenames, `nak_retry_us`, `WalWriter::append`,
   `byte 8`, `[Event; 10_000]`. Audit not yet run from
   Round 3 — readme hygiene is explicitly Round 4 scope.

2. **PROGRESS.md** — verify crate-status section reflects the
   12 current crates (no `rsx-maker`). The crate count was
   bumped to 12 in MEMORY.md but PROGRESS.md may still cite
   the older inventory.

3. **BLOG.md / ONEPAGER.md** — likely use marketing-style
   summaries that lag the v0.5.0 renames. Check for `CMP` /
   `DXS` / `cmp_send_breakdown_bench` (the latter renamed to
   `cast_send_breakdown_bench` in `specs/2/52-blog.md` during
   this round, but `BLOG.md` itself wasn't touched).

4. **`specs/2/22-perf-verification.md`** still cites the older
   `cmp_bench.rs` in some prose; the table row was fixed. A
   second pass may catch more.

5. **`specs/2/41-testing-matching.md`** test-file layout
   section (around line 200+) was not re-checked; the
   matching test layout likely also drifted from the actual
   `tests/` directory.

6. **`rsx-cast/ARCHITECTURE.md`** perf-table line for
   `casting RTT, loopback, 128 B = 11.26 µs` — number is
   from `cast_rtt_bench` (renamed from `cmp_rtt_bench`). The
   bench-name column may have been updated to `cast_*` but the
   numbers themselves should be re-verified by Round 4's
   "re-run every Criterion bench" step in the sprint plan.

7. **Cited commit hashes** — the new v0.5.1 changelog entry
   cites short hashes from this session's commits. Once
   Round 4 runs, audit for any other markdown that quotes
   commit hashes; verify they still resolve.

## LOC delta

```
specs/2/*.md                   +140 -130   (~+10 / mostly rewrites)
rsx-cast/specs/*.md             +52 -100   (-48 — major: dropped flow-control chapter)
rsx-cast/ARCHITECTURE.md         +14 -11    (+3)
rsx-matching/ARCHITECTURE.md      +6  -4    (+2)
rsx-marketdata/ARCHITECTURE.md    +1  -1     0
rsx-book/ARCHITECTURE.md          +5  -3    (+2)
rsx-cli/ARCHITECTURE.md           +1  -1     0
ARCHITECTURE.md (root)            +4  -4     0
docs/benches.md                   +9  -8    (+1)
CHANGELOG.md                     +70  -0   (+70 v0.5.1 entry)
TOTAL                          +302 -262   (+40 — but most files net-shorter; the v0.5.1 changelog is the only growth)
```

The cleanup deleted more spec prose than it added (drift was
verbose); only `CHANGELOG.md` grew on net.
