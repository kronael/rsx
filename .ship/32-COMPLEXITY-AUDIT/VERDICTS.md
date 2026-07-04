# Complexity Audit — VERDICTS (opus adjudication, 2026-07-04)

Verification pass over FINDINGS.md. No source edited. Each finding opened at
its cited lines; concrete claims (dead code, call sites, trait bounds,
duplication) re-verified by grep. Verdict vocabulary:

- **CONFIRM+SIMPLIFY** — real smell, fix is behavior-preserving. Risk tagged
  `trivial` (mechanical, local) or `needs-care` (invariant/hot-path/surface).
- **CONFIRM-BUT-KEEP** — complex but load-bearing (Chesterton's fence).
- **REJECT** — misread / not a smell / justified by copy-2-3 rule.
- **RECORD-ONLY** — rsx-cast is FROZEN. Technical merit stamped; NO action
  without explicit founder sign-off.

## Per-finding verdicts

A1 CONFIRM+SIMPLIFY (needs-care) — write_events_to_wal is test/bench-only (all 8 call sites in benches/tests; prod uses publish_events @ main.rs:748,1079); real ~190-line dup BUT not a pure superset: write_events_to_wal skips BBO ("derived on replay"), publish_events emits it via fan_out. Shared per-event record-build must preserve that one-arm divergence + bench semantics.
A2 CONFIRM+SIMPLIFY (needs-care) — main() ~725 lines with a ~212-line inline order-handler closure (603-815); named-fn precedent (process_cancel@979, emit_config_applied@936). Extract handle_order_message; preserve dedup/WAL/dual-fan-out order + latency sample points.
A3 CONFIRM+SIMPLIFY (trivial) — update_order_index copy-pasted as update_order_index_local (wal_integration.rs:665), comment admits it. Same crate → make main's pub(crate), delete the copy.
A4 CONFIRM+SIMPLIFY (needs-care) — emit_bbo (rsx-book/matching.rs:246) is private and re-derived inline in process_cancel; cross-crate so needs `pub` (not pub(crate)) — widens rsx-book API. Behavior-preserving.
A5 CONFIRM+SIMPLIFY (trivial) — get_env_u32/u8/i64 identical modulo return type; fold to `get_env<T: FromStr>`. Same error kinds/messages.
A6 CONFIRM+SIMPLIFY (trivial) — pct_5/15/30/50 differ only by literal; `pct(mid,n,denom)` ×4. Runs once per recenter, not hot.
A7 CONFIRM-BUT-KEEP — advance_frontier_to (target-driven, single side) vs migrate_batch (batch-budget, both sides + zero-floor guard + complete_migration) have MATERIALLY DIFFERENT termination; the loops are load-bearing, not mergeable. Only a 1-tick step helper is extractable (marginal value/risk).
A8 CONFIRM+SIMPLIFY (needs-care) — scan_next_bid/ask ~20-line dup differing by sentinel/side-filter/comparator; `scan_next(Side)`. O(n) scan itself justified (fallback path); only the 2 copies are the smell.

B1 CONFIRM+SIMPLIFY (needs-care) — handle_connection ~660 lines (35-695), all stages inline. Extract handle_new_order/handle_cancel/run_heartbeat. Gateway on critical path; monoio single-thread, behavior-preserving.
B2 CONFIRM+SIMPLIFY (needs-care) — run_main ~761 lines; 3 near-identical Faulted/Reconnect drain blocks. Factor drain_recv<F>. MUST preserve fills(me)>orders(gw)>mark ordering invariant (documented main.rs:520-523).
B3 CONFIRM+SIMPLIFY (needs-care) — process_fill taker(264-310)/maker(313-364) near-dup differing only by user-id/side/fee_bps/log. settle_side(...) helper; preserve update order + error-return semantics on the risk hot path.
B4 CONFIRM-BUT-KEEP — positions_by_user/frozen_by_user are a live O(all)->O(per-user) secondary index (read on the per-fill hot path @478, iter_positions_for_user@231), maintained in lockstep. Keep the index. Optional choke-point wrapper is discretionary abstraction (code already asserts no-drift @212-214) — not a pure win; skip unless drift bug appears.
B5 CONFIRM+SIMPLIFY (trivial) — two rand_jitter diverged (main.rs %1_000_003 vs persist.rs %1000), same crate. Single rand_jitter(modulus) in risk_utils; each caller keeps its modulus (distributions differ).
B6 CONFIRM-BUT-KEEP — gateway single-shot skip-to-gap vs risk 5×15ms retry is a DELIBERATE, documented architectural split: gateway is downstream and recovers via risk re-emit (doc @57-62; "in-flight fills lost" @86), risk is authoritative and fail-loud-panics. Intent is settled in code → resolves the DEFER question as KEEP. Do NOT merge.
B7 CONFIRM+SIMPLIFY (needs-care) — reload_symbol_overrides reimplements the RSX_SYMBOL_{id}_* parse-and-assign idiom per crate over DISJOINT field sets. Only the idiom dedups (env_override<T> helper, couples with B10); keep the crate split + gateway's sid-bounds guard. Low urgency.
B8 CONFIRM+SIMPLIFY (trivial) — gateway ws_read_frame (ws.rs:241) is DEAD: zero callers repo-wide, not re-exported (lib.rs only exports drain_replay), only doc ref ARCHITECTURE.md:120. (marketdata's same-named fn is a separate crate, live.) Delete + fix the doc line. PURE WIN.
B9 CONFIRM+SIMPLIFY (trivial) — `if mark_dirty { rebuild_fallback() }` guard ×3 (459,582,1125). Internalize as ensure_fallback_fresh() checking the flag.
B10 CONFIRM+SIMPLIFY (trivial) [SEVERITY DISAGREEMENT] — env_u32/u64/usize byte-identical across FOUR prod crates (gateway/risk/mark/marketdata) + 4 bench copies, not 2. The finding's own "3rd crate → extract" trigger is already met; extract to rsx-types::env_utils. Behavior-preserving.

C1 RECORD-ONLY (merit: real, needs-care) — try_recv_with ~250-line match-in-loop, 5+ nesting; inline deliver(943-952) & reorder-drain(1034-1059) share delivery logic (advance expected_seq, clear nak_sent_at, return Data) from two byte-sources. Decomposition only; drain path also clears reorder_seqs[slot]. FROZEN.
C2 RECORD-ONLY (merit: real) — send/send_framed duplicate the ring-vs-buf branch + ring_seqs/ring_lens bookkeeping verbatim; differ by byte-source and next_seq update (+=1 vs =seq+1, must stay per-caller). FROZEN.
C3 RECORD-ONLY (merit: real, but propagation ≠ refactor) — 3 read-header/CRC loops; skip-corrupt `continue` exists ONLY in scan_file_for_seq(758-762). WalReader::next / scan_file_seq_range intentionally STOP at first corrupt (torn-tail = end-of-log). Propagating `continue` would CHANGE replication semantics, not dedup — flag: not a safe mechanical fix even if unfrozen. FROZEN.
C4 RECORD-ONLY (merit: HIGH, cleanest) — 3 hand-rolled `from_raw_parts` (repl_client:234, repl_server:173,225) are byte-exactly `encode_utils::as_bytes` (exists, pub, re-exported lib.rs:23). Trivially behavior-preserving swap — the single frozen finding most worth pitching for sign-off. FROZEN.
C5 RECORD-ONLY (merit: readability) — magic `expected_seq - seq > 100` (922) unnamed while neighbors use named consts w/ rationale. `const SENDER_RESET_GAP=100` + comment. FROZEN.
C6 RECORD-ONLY (KEEP-lean) — CastRecv/CastRecvWith Faulted/Reconnect overlap is a DELIBERATE owned-vs-zero-copy split (CastRecvWith delivers via FnOnce, no heap; try_recv is the allocating shim). Merging = API change. FROZEN.
C7 RECORD-ONLY (merit: minor) — rotate() File::create("/dev/null") is a mem::replace ownership placeholder; Option<File>+take() is cleaner but a struct-field change. FROZEN.
C8 RECORD-ONLY (merit: minor, ≠ refactor) — prepare() returns io::Result yet asserts! on oversized record. Converting to Err changes the failure contract (panic→recoverable), a behavior change callers may rely on. FROZEN.

D1 CONFIRM+SIMPLIFY (needs-care) — VERIFIED: ReplicationConsumer::run_once<F> (repl_client.rs:136) has `F: FnMut(RawWalRecord)->bool`, NO 'static bound. The raw-pointer + `unsafe{&mut *}` dance in replay.rs:134-198 is unnecessary; a plain &mut-capturing closure compiles. Removes unsafe. Caller-side (rsx-cast untouched). Compile-check required.
D2 CONFIRM+SIMPLIFY (needs-care) [overlaps logged CLI-PTR-READ-UNALIGNED-UB] — decode_payload (~400-line match) uses `std::ptr::read` on align-1 &[u8] ptr = potential UB. Safe `rsx_cast::decode_payload::<T>` (read_unaligned + len-guard) exists. Per-arm swap fixes the UB AND dedups; keep the per-record format!/json!. This is a CORRECTNESS fix, not just complexity.
D3 CONFIRM+SIMPLIFY (trivial) — VERIFIED ZERO call sites: `impl Default for LoadGauges` (220-244, ~24 dup lines); every site uses LoadGauges::new(); no Default::default() inference. Delete. PURE WIN.
D4 CONFIRM+SIMPLIFY (needs-care) — main() ~370 lines. Pre-runtime setup (config/bind/pin/health) cleanly extractable; event loop constrained by monoio Rc<RefCell> !Send co-location — keep spawn/borrow topology. Lower priority.
D5 CONFIRM+SIMPLIFY (needs-care) — handle_insert/cancel share borrow→lookup→broadcast; handle_fill adds a trade fan-out. apply_and_broadcast helper for the common core; fill can't fully collapse. Low priority.
D6 CONFIRM+SIMPLIFY (needs-care) — aggregate-drain & staleness-sweep dup prepare→append→send; sweep OMITS gauges.publishes.fetch_add (real metric undercount). Shared publish_event helper; note unifying also FIXES the metric (behavior change) — decide if the omission was intentional. Off-path, low priority.
D7 CONFIRM+SIMPLIFY (trivial) — has_bbo/depth/trades identical but for the mask; has_channel(client,symbol,mask) + 3 wrappers.
D8 CONFIRM+SIMPLIFY (trivial) — aggregator "Same as aggregate/sweep_stale" comments reference DELETED fns (only *_with_staleness exist). Fix comments; optional suffix-drop rename touches 2 call sites (main.rs:234,271).
D9 CONFIRM+SIMPLIFY (trivial) — http_200/503/404 dup the response template; http_response(status_line,body) + 404's fixed body. Identical byte output.
D10 CONFIRM+SIMPLIFY (needs-care) — extract_symbol_id hardcodes payload[16..20] for 3 types, then the same payload is typed-decoded ~25 lines later (double-decode + silent-break-if-layout-shifts). Reuse one decode_payload::<T>().symbol_id; preserve the gap-detection-before-apply ordering. Off critical path.

## Verdict counts

- CONFIRM+SIMPLIFY: 25 (A1,A2,A3,A4,A5,A6,A8, B1,B2,B3,B5,B7,B8,B9,B10, D1,D2,D3,D4,D5,D6,D7,D8,D9,D10)
- CONFIRM-BUT-KEEP: 3 (A7, B4, B6)
- REJECT: 0
- DEFER: 0 (B6, the nominated DEFER, resolves to KEEP — intent is documented in code)
- RECORD-ONLY (rsx-cast, frozen): 8 (C1–C8)

## Apply first — ranked (behavior-preserving, non-frozen, value/risk)

PURE WINS (do these first — dead-code deletion, zero behavioral surface):
1. **D3** — delete dead `impl Default for LoadGauges` (~24 lines, 0 call sites).
2. **B8** — delete dead gateway `ws_read_frame` (~90 lines, 0 callers) + drop ARCHITECTURE.md:120 mention.
3. **A3** — delete admitted copy-paste update_order_index_local; make main's pub(crate). Same crate, trivial.

HIGH VALUE (fixes a real defect, worth the slightly larger diff):
4. **D1** — remove the unnecessary `unsafe` raw-pointer block in marketdata replay (run_once has no 'static bound). Deletes UB-adjacent unsafe; compile-check gate.
5. **D2** — swap rsx-cli decode_payload arms to safe `rsx_cast::decode_payload::<T>`. Closes logged CLI-PTR-READ-UNALIGNED-UB. Mechanical per-arm across ~400 lines.

EASY DEDUP (trivial, low risk, small diffs):
6. **B10** — hoist env_u32/u64/usize into rsx-types::env_utils (4-crate dup).
7. **A5** — generic get_env<T: FromStr>.
8. **B5** — single rand_jitter(modulus) in risk_utils.
9. **B9** — ensure_fallback_fresh() internalizing the mark_dirty guard.
10. **D7 / D9 / A6 / D8** — has_channel / http_response / pct() / stale-comment fix.

DEFER-TO-LATER (needs-care structural refactors on hot/critical paths — real value, do deliberately with tests):
11. **B3** (settle_side), **A2** (handle_order_message), **B2** (drain_recv<F>), **B1** (gateway handler split), **A8**, **A4**, **D4**, **D5**, **D6**, **D10**, **A1** (BBO-divergence caveat), **B7** (couples with B10).

FROZEN — pitch for sign-off, do not touch unprompted:
- **C4** is the one rsx-cast finding that is a clean, behavior-preserving pure win (3× from_raw_parts → existing as_bytes). If any C item is worth a founder ask, it's this. C5 (named const) is the next-cheapest. C3/C8 are NOT safe mechanical refactors even unfrozen (they change failure/corruption semantics).

## Disagreements with the sonnet sub's severity

- **B10** — sub graded LOW / "leave, not urgent (2-3 copy rule)." WRONG premise: the dup is already across 4 prod crates, so the finding's own extraction trigger is met. Upgrade to a should-do CONFIRM+SIMPLIFY.
- **D2** — sub graded HIGH as complexity. Under-weights that it fixes a *logged UB correctness bug*; it should rank as the highest-priority actionable item, above pure-complexity HIGHs.
- **C3** — sub framed as "a fix in one didn't propagate" (implying: propagate it). Flag: WalReader::next / scan_file_seq_range stop-at-corrupt is intentional torn-tail semantics; propagating the skip would silently change replication behavior. The smell is real but the implied fix is not a safe refactor.
- **A1** — sub graded HIGH and implied easy collapse/delete. The BBO-persistence divergence (write path skips BBO, publish path emits it) means it is not a pure superset; risk is higher / ease lower than implied.
- **A7** — sub "LOW, load-bearing maybe." Confirmed load-bearing: the loop terminations genuinely differ; the loop-merge is unsafe and only a marginal step-helper is extractable. Effectively KEEP, lower actionability than a MED/LOW dedup.
