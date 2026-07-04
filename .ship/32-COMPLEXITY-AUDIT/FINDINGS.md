# Complexity Audit — 2026-07-04 (sonnet pinpoint → opus verify)

4 read-only sonnet subs over all crates. Governing principle (CLAUDE.md Boring
Code): code should be rather simple; complexity is a smell → simplify unless it
is load-bearing (Chesterton's fence). Opus verifies each below.

**rsx-cast (C*) is FROZEN — record-only. No edits without explicit sign-off.**

## A — Hot path (rsx-book, rsx-matching)
- A1 [HIGH] `rsx-matching/wal_integration.rs:54-245` vs `249-463` — `write_events_to_wal` duplicates `publish_events` ~190 lines (same 6 Event arms, byte-identical record builds). write_events_to_wal is TEST/BENCH-ONLY (prod uses publish_events at main.rs:748,1079). Fix: extract per-event record-build → Framed shared by both, or delete write_events_to_wal and have benches use publish_events w/ loopback senders. Load-bearing: no.
- A2 [HIGH] `rsx-matching/main.rs:210-934` — `main()` ~700 lines incl a 200-line inline order-handler closure (603-815: dedup+WAL+dual fan-out+latency+order-index). Precedent exists (`process_cancel`,`emit_config_applied` are named fns). Fix: extract `handle_order_message(&mut ctx, hdr, payload)` with a context struct. Load-bearing: no for structure (content invariants preserved).
- A3 [MED] `rsx-matching/main.rs:65-98` vs `wal_integration.rs:665-698` — `update_order_index` copy-pasted as `_local` (comment admits it). Fix: make main's `pub(crate)` (or move to wal_integration) + delete dup. Load-bearing: no.
- A4 [MED] `rsx-book/matching.rs:246-273` (`emit_bbo` private) vs `rsx-matching/main.rs:1046-1076` — BBO lookup+emit re-derived in process_cancel because emit_bbo isn't pub. Fix: `pub(crate)` emit_bbo + call it. Load-bearing: no.
- A5 [LOW-MED] `rsx-matching/main.rs:148-191` — `get_env_u32/u8/i64` triplicated. Fix: one generic `get_env<T: FromStr>`. Load-bearing: no.
- A6 [LOW] `rsx-book/compression.rs:22-76` — pct_5/15/30/50 = 4 near-identical checked_mul/div/unwrap blocks. Fix: `pct(mid,n,denom)` ×4. Runs once per recenter (not hot). Load-bearing: no.
- A7 [LOW] `rsx-book/migration.rs:102-126` vs `221-255` — advance_frontier_to & migrate_batch reimplement per-side tick-step. Fix: shared step_bid/step_ask helper (callers keep own stop cond). Load-bearing: maybe (differing loop termination).
- A8 [LOW] `rsx-book/book.rs:377-421` — scan_next_bid/scan_next_ask ~20-line near-dup (side+comparator differ). Fix: `scan_next(side)` with inline match. Load-bearing: partial (O(n) scan itself justified; only the 2 copies are the smell).

## B — Risk + gateway
- B1 [HIGH] `rsx-gateway/handler.rs:35` — `handle_connection` ~660-line async fn (handshake+register+heartbeat+NewOrder/Cancel/HB/Error dispatch inline). Fix: extract handle_new_order/handle_cancel/run_heartbeat_cycle. Load-bearing: no.
- B2 [HIGH] `rsx-risk/main.rs:279` — `run_main` ~760 lines (boot + steady loop w/ 3 near-identical Faulted/Reconnect/Empty blocks for me/gw/mark). Fix: boot_and_promote()->LiveContext + run_live_loop(ctx); factor drain_recv<F>(). Load-bearing: partial (fills>orders>mark ordering invariant — preserve).
- B3 [MED] `rsx-risk/shard.rs:248` — `process_fill` two ~55-line taker/maker near-dup blocks. Fix: `settle_side(user,symbol,side,price,qty,seq,fee_bps)->i64`. Load-bearing: no.
- B4 [MED, LOAD-BEARING] `rsx-risk/shard.rs:44,62` — positions_by_user/frozen_by_user secondary per-user index maintained in lockstep across ~5 sites each. Fix: NOT removal (it's the O(all)->O(per-user) perf fix); wrap in a type with one insert/remove choke point. Load-bearing: yes.
- B5 [LOW-MED] `rsx-risk/main.rs:270` & `persist.rs:508` — two `rand_jitter()` fns diverged. Fix: single `rand_jitter(modulus)` in risk_utils.rs. Load-bearing: no.
- B6 [LOW-MED] `rsx-gateway/main.rs:63` vs `rsx-risk/failover.rs:45` (both `handle_replay`) — risk retries 5x15ms, gateway single-shot skip-to-gap (documented in-flight-fill-loss tradeoff). Fix: confirm gateway's simpler behavior is intentional before merging. Load-bearing: maybe (verify intent).
- B7 [LOW] `rsx-risk/shard.rs:849` vs `rsx-gateway/state.rs:91` (`reload_symbol_overrides`) — RSX_SYMBOL_{id}_* pattern reimplemented per crate. Fix: env_override<T>() helper (keep crate split). Load-bearing: maybe (crate boundary intentional).
- B8 [LOW] `rsx-gateway/ws.rs:241` (`ws_read_frame`) — apparent DEAD CODE, ~90-line near-dup of ws_read_frame_buf (310, the one called). Fix: delete after confirming no callers/tests. Load-bearing: no.
- B9 [LOW] `rsx-risk/shard.rs:459-461,582-584,1125-1127` — `if mark_dirty { rebuild_fallback() }` guard ×3. Fix: `ensure_fallback_fresh()` checking flag internally. Load-bearing: no.
- B10 [LOW] `rsx-risk/config.rs` & `rsx-gateway/config.rs` — env_u32/usize/u64/str byte-identical per crate. Fix: leave (2-3 copy rule) unless a 3rd crate dups → rsx-types::env_utils. Load-bearing: no (not urgent).

## C — Transport (rsx-cast) — RECORD-ONLY, FROZEN, sign-off required
- C1 [HIGH] `cast.rs:812-1061` — `try_recv_with` ~250-line match-in-loop, 5+ nested decision points; reorder-drain (1034-1059) dup of inline deliver (943-952). Fix: extract deliver_in_order/buffer_out_of_order/resync_if_needed (decomposition only). Load-bearing: logic yes, monolith shape no.
- C2 [MED] `cast.rs:245-299` vs `310-348` — send() & send_framed() duplicate ring-vs-buf branch verbatim. Fix: private write_and_send(seq,header,payload). Load-bearing: no.
- C3 [MED] `wal.rs:631-676,726-777,415-475` — 3 near-identical read-header/validate/read-payload/verify-CRC loops (scan_file_seq_range, scan_file_for_seq, WalReader::next); a fix in one (skip-corrupt at 758-762) doesn't propagate. Fix: shared read_frame(&mut File)->Option<RawWalRecord>. Load-bearing: no.
- C4 [LOW-MED] `replication_client.rs:234-240`, `replication_server.rs:173-179,225-231` — hand-rolled unsafe from_raw_parts ×3 when `encode_utils::as_bytes` exists. Fix: use as_bytes(). Load-bearing: no.
- C5 [LOW] `cast.rs:922-934` — magic `expected_seq - seq > 100` unnamed/underived (neighbors have rationale comments). Fix: named const + comment. Load-bearing: behavior yes, magic-number no.
- C6 [LOW] `cast.rs:556-597` — CastRecv/CastRecvWith identical variant sets (Faulted/Reconnect dup shape). Fix: maybe parametrize; low priority given zero-copy split intent. Load-bearing: maybe (hot-path alloc avoidance).
- C7 [LOW] `wal.rs:265-268` — rotate() uses File::create("/dev/null") placeholder for borrow-ck. Fix: file: Option<File> + take(). Load-bearing: no (but struct-field change).
- C8 [LOW] `wal.rs:148-178` (`prepare`) — assert! panic on oversized record vs io::Result used elsewhere. Fix: return Err(InvalidInput). Load-bearing: maybe (loud-fail-in-CI intent).

## D — Off-path (marketdata, mark, recorder, cli, health, log, tui, types)
- D1 [HIGH] `rsx-marketdata/replay.rs:134-198` — raw-pointer casts + unsafe deref in run_once closure, justified by a false claim; `ReplicationConsumer::run_once<F>` has NO 'static bound → plain FnMut compiles with zero unsafe. Fix: capture &mut locals normally, delete unsafe. Load-bearing: no.
- D2 [HIGH] `rsx-cli/main.rs:331-730` — `decode_payload` ~400-line match, each arm hand-rolls `std::ptr::read` (UB-prone, unaligned) + parallel format!/json!. (Overlaps logged CLI-PTR-READ-UNALIGNED-UB.) Fix: use `rsx_cast::decode_payload::<T>` per arm. Load-bearing: no.
- D3 [MED] `rsx-health/lib.rs:220-244` — `impl Default for LoadGauges` duplicates all 17 initializers from new(); ZERO call sites. Fix: delete the Default impl. Load-bearing: no.
- D4 [MED] `rsx-marketdata/main.rs:145-514` — main() ~370 lines (bootstrap+bind+pin+health+event loop). Fix: extract bootstrap_from_replay/build_cast_receivers/spawn_health/run_tick. Load-bearing: partial (monoio Rc<RefCell> co-location).
- D5 [MED] `rsx-marketdata/main.rs:541-657` — handle_insert/cancel/fill dup borrow→lookup→broadcast skeleton. Fix: apply_and_broadcast(state,symbol,max,f). Load-bearing: no (low priority).
- D6 [MED] `rsx-mark/main.rs:218-298` — aggregate-drain (233-256) & staleness-sweep (270-289) dup prepare→append→send; sweep omits gauges.publishes (metrics asymmetry). Fix: publish_event(...) shared. Load-bearing: no.
- D7 [LOW] `rsx-marketdata/subscription.rs:98-135` — has_bbo/has_depth/has_trades = 3 copies differing by mask. Fix: has_channel(client,symbol,mask). Load-bearing: no.
- D8 [LOW] `rsx-mark/aggregator.rs:106-136` — `_with_staleness` variants + "same as X" comments, but the plain aggregate/sweep_stale no longer exist. Fix: rename to aggregate/sweep_stale, drop rotted comments. Load-bearing: no.
- D9 [LOW] `rsx-health/lib.rs:344-385` — http_200/503/404 = 3 copies of the response template. Fix: http_response(status_line,body). Load-bearing: no.
- D10 [MED] `rsx-marketdata/main.rs:516-539` (`extract_symbol_id`) — hardcodes payload[16..20] for 3 record types (unenforced wire coupling; silent break if layout shifts). Fix: decode_payload::<T> + .symbol_id (already used 2 lines later). Load-bearing: maybe (avoids double-decode; off critical path).
