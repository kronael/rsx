# Bug queue

The review queue: **OPEN** and **DEFERRED** items only. Resolved bugs live
in git (commit refs below) and `CHANGELOG.md` — not here.

## Status — 2026-07-08 — book+matching refine critique (merged from .matching-book-refine)

Correctness findings from the refine-pass critique, confirmed still present in
main. The minimization/doc findings were applied (`b2d3c52`).

- **ME-FILL-DONE-STALE-SLOT-TIF** (MED-HIGH, correctness) — `emit_events`
  (`rsx-matching/src/wal.rs`, Fill + OrderDone paths) reads
  `book.orders.get(handle)` for `(reduce_only, tif)` **after** the match cycle.
  In `rsx-book/src/matching.rs` a fully-filled maker's slot is `free()`d
  mid-cycle, and a GTC taker with residual then `insert_resting` pops that same
  slot off the LIFO freelist — so for "GTC taker fully fills ≥1 maker, residual
  rests" the maker's `FillRecord`/`OrderDoneRecord` carry the **taker's**
  `tif`/`reduce_only` on the wire. Fix: capture `tif`/`reduce_only` into
  `Event::Fill`/`Event::OrderDone` at emission time (slot live), drop the
  post-cycle derefs.
- **MD-SHADOW-MIXED-SLOT-BBO-L2** (MED, correctness — marketdata) — the shadow
  book reimplements BBO/L2 with the FIFO-head anti-pattern rsx-book fixed:
  `rsx-marketdata/src/shadow.rs` `derive_bbo` (~:129) + `side_levels` use
  `level.total_qty`/`order_count`/`head.price`, which in a mixed compressed
  slot drops a side / misattributes qty. Fix: `derive_bbo` → `book.current_bbo()`;
  `side_levels` → `bid_count`/`ask_count` per raw price.
- **BOOK-USER-RECLAIM-UNWIRED-U16-CAP** (LOW-MED, liveness) — user reclamation
  (`rsx-book/src/user.rs:95` `try_reclaim`) has no production caller, so
  `get_or_assign_user`'s `user_bump` hits the u16 cap at 65 535 distinct users
  per book → panic/wrap over a long run. Wire `try_reclaim` into the ME idle
  loop, or delete it and document the cap.

## Status — 2026-07-08 — live latency legs not stamped (rsx-term speed strip)

Design settled → **`specs/2/59-latency-observability.md`** (planned). The
live internal/engine legs are delivered by **per-hop timestamps embedded in
the normal records** (not a bespoke delta frame), derived under assumed clock
sync (PTP in prod, single-box in dev); the client `net` leg stays
client-measured. Aggregate latency is Prometheus via the existing
`33-telemetry.md` Vector pipeline, not hand-rolled. The two tasks below are
now increments of spec 59:

- **GW-STAMP-LATENCY-LIVE** (feature, spec 59 inc 1-2) — gateway stamps
  `gw_in_ns`/`gw_out_ns` into the returned record (generalising
  `taker_ts_ns`); today it only logs a `latency_sample!` trace
  (`rsx-gateway/src/handler.rs:301`). With the risk/ME stamps present the
  terminal derives `internal = gw_out − gw_in` itself.
- **ME-ENGINE-LATENCY-NOT-REPORTED** (feature, spec 59 inc 2) — ME stamps
  `me_in_ns`/`match_done_ns` (reusing the `latency-trace` per-stage
  measurement — a within-host duration, no clock-sync needed for this leg);
  risk stamps `risk_in_ns`. Terminal derives `engine = match_done − me_in`.

## Status — 2026-07-08 — rsx-matching release CTO review (see .ship/41-MATCHING-RELEASE/CTO-REPORT.md)

Record-only (bug-triage protocol): findings from the release-pass review.
Full evidence + verdicts in the report; the top item was verified against
code. Do NOT fix without founder go.

- **ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD** (MED-HIGH, correctness) — **FIXED
  2026-07-08** (`4a0df9d`): recovery now rebuilds the full 300 s dedup window
  by scanning the WAL for `RECORD_ORDER_ACCEPTED` records within the window
  and seeding `DedupTracker` with each entry's remaining TTL
  (`DedupTracker::seed` + `wal::rebuild_dedup_window`), reusing the WAL as the
  single source of truth (no new snapshot format). Integration test
  `dedup_window_rebuilt_from_wal_for_pre_snapshot_order` proves a pre-snapshot
  order's resend is deduped after recovery. Original defect: dedup was
  not reconstructed across a snapshot boundary, so a post-crash client
  resend could double-execute. `rebuild_order_index_from_book`
  (`rsx-matching/src/main.rs:98`) restores the order index from the slab but
  not `dedup`; `replay_wal_after_snapshot` only replays
  `RECORD_ORDER_ACCEPTED` for `seq >= start_seq` (post-snapshot). Snapshot
  cadence ~10 s, dedup window 300 s (`dedup.rs:6`) — so any order accepted
  >~10 s before a crash loses dedup on restart, and a legitimate resend of
  its `cid` within 5 min is treated as new → double-fill. Violates the
  exactly-one-completion invariant. **Known in code** (TODOs at
  `main.rs:97,313-318`) but was never in BUGS.md until now. Fix (needs
  design, NOT a coding-agent task yet): persist a dedup snapshot alongside
  the book snapshot, or widen replay to cover the dedup window.
- **ME-NEXT-SEQ-REGRESSION** (LOW, traceability) — the seq-regression guard
  (`main.rs:346-357`) is correct, but its `bugs.md` code citation points at
  a non-existent entry (this one). Filed for traceability; sweep the repo
  for other orphaned `bugs.md` citations.
- **ME-REASON-DUPLICATE-NAMESPACE** (LOW, correctness-latent) —
  `REASON_DUPLICATE` (=3, `main.rs:52`) is disconnected from rsx-book's
  `FAIL_*` enum (0-2, `event.rs:23-25`); a future `FAIL_*`=3 would silently
  alias "duplicate" in `OrderFailedRecord.reason`. Fix: one shared enum.
- **ME-WAL-WRITER-DUPLICATION** (MED, duplication → drift) — **FIXED
  2026-07-08** (`761aa94`). Two corrections to the original finding: (1) the
  "~200-line near-duplicates" framing was already stale — the record-build
  match lived in one place (`emit_events`); both public fns were 7-line
  sink-picker wrappers, and the ONLY divergence was `WalSink::emit_bbo`
  no-oping on `Event::BBO`. Fix removed the `emit_bbo` special method so BBO
  flows through the shared `emit()` in both sinks. (2) The "understates the
  266 ns accept" claim was WRONG for that bench: production always persisted
  BBO (`FanoutSink` used the default), and the flagship accept scenario (1-lot
  IOC into a deep, non-draining level) never moves the best price, so
  `matching.rs:232` never fires `emit_bbo` → no BBO record to skip or write.
  266 ns is honest for its scenario. See ME-ACCEPT-BENCH-NO-BBO below.
- **ME-ACCEPT-BENCH-NO-BBO** (LOW, bench representativeness) — the flagship
  `me_accept_path/full` (266 ns) uses a scenario that never moves the BBO, so
  it doesn't pay the one extra BBO WAL-record write a BBO-moving accept (level
  drain, or a resting residual creating best-bid) would. Not a wrong number —
  honest for its scenario — but the headline may not represent a BBO-moving
  accept. **RESOLVED 2026-07-08 — relabeled** (founder chose relabel over
  scenario redesign): the 266 ns headline now reads "one fill, BBO unchanged"
  across README, rsx-matching README/ARCHITECTURE, and the Jul-03 report;
  a BBO-moving accept adds one WAL record.
- **ME-CANCEL-REIMPL** (LOW, duplication) — `process_cancel`
  (`main.rs:955`) reimplements the drift-check inline instead of
  `rsx-book::cancel_order_checked` (`book.rs:355`), missing its
  capacity-bound check.

## Status — 2026-07-08 — ME→GW-direct spec is NOT ship-ready (codex adversarial audit)

- **ME-GW-DIRECT-SPEC-GAPS** (design blocker — do NOT hand to a coding agent
  yet) — the async-output-split in `specs/2/28-risk.md` (ME sends fill straight
  to gateway; settle to Risk async) is `status: partial`/not-implemented, and an
  adversarial audit found it underspecified and unsafe as written. Frontmatter
  says `status: shipped` (`28-risk.md:1`) but the design is partial
  (`28-risk.md:201`) — fix the frontmatter too. Verdict: **needs-spec-work-first**.
  Gaps that must be closed BEFORE any implementation:
  1. **No authoritative client sequence** once ME (per-symbol seq) and Risk
     (risk-gw seq) both feed the gateway — two clients/replay can disagree on
     order. Today Risk restamps one contiguous stream (`failover.rs:123`).
  2. **Reject edge contradiction** — spec says the Risk→GW insufficient-margin
     reject "cannot be removed" (`28-risk.md:173`) but also says Risk drops
     `forward_to_gw`/response ring (`28-risk.md:206`); `forward_to_gw` carries
     BOTH fills and rejects (`rsx-risk/src/main.rs:821`). Drop it and rejects
     vanish → client can't tell rejection from transport loss.
  3. **Gateway replay for the ME→GW stream is undefined** — no
     `RSX_ME_GW_REPLICATION_ADDR`, no per-symbol tip/replay-source/dedup; today
     gateway replays only Risk (`main.rs:87`). A dropped direct fill never
     reaches the client.
  4. **ME live-stream contents under-specified** → false FAULTED gaps: if the
     gateway gets only fill/DONE/FAILED, the WAL-seq holes at ORDER_ACCEPTED/BBO
     look like loss.
  5. **Duplicate fills** — no dedup key (`fill_id` / `(symbol,seq)`); during any
     mixed rollout the client double-counts (`route.rs:34` pushes blind).
  6. **Reduce-only solvency — FALSE POSITIVE (2026-07-08).** ME *does* hold
     an authoritative per-symbol position (`net_qty`, `rsx-book/src/user.rs`)
     and clamps/rejects reduce-only against it synchronously at match time
     (`matching.rs:59-94`); the view is never stale (single-threaded, in-order
     fills). The posited long-1 → double-reduce → naked-short cannot occur: the
     second reduce-only hits `net_qty==0` → `FAIL_REDUCE_ONLY`. Risk's `Ok(0)`
     is correct (reduce frees margin; enforcement is ME's — `28-risk.md:351`,
     `47-validation-edge-cases.md:145`). Position recovery is exact (snapshot
     `net_qty` + replay through `process_new_order`). No fix.
  7. **Liquidation race — FALSE POSITIVE (2026-07-08).** `UserInLiquidation`
     is checked synchronously at Risk pre-flight on the next order; the only
     stale-position hazard was reduce-only, which ME owns (see 6). No
     independent hole.
  8. **Exactly-one-completion** at risk — direct ORDER_DONE clears pending, a
     late Risk ORDER_FAILED emits a second terminal (`route.rs:105`).
  **Settled 2026-07-08.** The acceptance-commits invariant — once Risk
  pre-flight accepts, the fill is guaranteed; Risk only *processes* fills, it
  never vetoes — dissolves gaps 1, 2, 8: per order exactly one terminal
  producer (Risk iff pre-flight-rejected, else ME), no cross-stream ordering
  conflict, no late Risk terminal. Gaps 6, 7 are false-positives (above).
  Remaining work is **mechanical transport only**: gateway as a full
  ME-replication consumer (3, 4) + `fill_id` dedup (5). Still needs the spec
  rewritten around the invariant + the transport story before implementation,
  but it is a bounded feature, not the high-risk rewrite the audit implied.
  See ME-HOLDS-USER-STATE for the related edge-buffer direction.

- **ME-HOLDS-USER-STATE** (design, OPEN/DEFERRED — record-only) — ME
  (`rsx-book`) holds two pieces of per-user state: `net_qty` (per-symbol
  position, for the reduce-only clamp, `user.rs`) and the `DedupTracker`
  (`(user_id, order_id)` idempotency, `dedup.rs`). Neither is an account — no
  balances/margin/collateral (those are Risk's, which is *why* Risk shards by
  user: portfolio margin ME structurally can't compute). The rule: **ME holds
  exactly the per-user state whose decision must be made at the match sequence
  point** — position-as-of-match for reduce-only, already-seen for dedup — the
  truths Risk's async view can't provide in time. Cost: it drags in per-symbol
  slab-GC for idle users (`order_count`/`zero_since_ns`/`RECLAIM_GRACE_NS`).
  Open question: can the edge hold *less*? Direction is the generalization —
  a pre-authorized risk **buffer** at the ME edge (Risk grants a conservative
  per-symbol margin allowance; ME draws down synchronously; Risk tops-up /
  revokes async), which would also let the *forward* Risk hop be skipped
  (`GW→ME→GW`). Hard part = buffer sizing + cross-symbol partitioning +
  revocation as mark moves; not the hot path. State is isolated behind
  `UserRegistry` (`rsx-book/src/user.rs`) as a clean seam to change/remove.
  Unsolved.

## Status — 2026-07-08 — eager recenter is a tail-latency spike (by design, revisit)

- **RECENTER-EAGER-TAIL-SPIKE** (MED, latency/design — not a correctness bug) —
  `maybe_recenter` → `Orderbook::recenter_now` migrates the whole book
  **eagerly in one shot** (`rsx-book/src/migration.rs:254`,
  `complete_migration_eager:236`). Cost is O(old slot count ≈ 617k for
  BTC-PERP): it scans the old slot array, skips empty levels O(1), and remaps
  the occupied ones — bounded by slot count, not order count, but still a
  spike far above a 60 ns match, landing on whichever order triggers a >2.5%
  mid drift (i.e. during volatility, when latency matters most). **Why eager:**
  a lazy per-order `resolve_level(price)` scheme would let a marketable order's
  crossing prices lie outside a partially-migrated band, so a half-migrated
  book would miss crossing liquidity and break price-time priority
  (`recenter_now` contract, `migration.rs:247`). The incremental
  `migrate_batch` frontier walk exists but is test/non-live-only. **Fix (to
  remove the spike):** a correct lazy/incremental migration for live matching —
  either migrate only the crossing band eagerly and rest lazily on idle, or
  make the matcher consult both old+new maps safely during the migration
  window. Non-trivial ME change; needs a design + adversarial review before
  code. Until then the engine trades a rare bounded spike for the correctness
  guarantee. Concept page (`docs/concepts/05-slab-and-compression.md`) documents
  the tradeoff honestly.

## Status — 2026-07-08 — stale latency/size numbers across docs (concepts CTO review)

Surfaced while reconciling `docs/concepts/*` to the code. Both are
doc-vs-doc / doc-vs-code number drift, not runtime defects. The concept
pages were corrected to the code; the *authoritative* docs below still
carry the stale figures. Record-only.

- **DOC-RT-FLOOR-DRIFT** (LOW, docs) — the in-process round-trip floor is
  stated three ways: `README.md:234` and the concept index say 7.5 µs p50 /
  16.9 µs p99 ("measured"); `docs/benches.md:128` has `bench-match-rt` at
  9.58 µs; `reports/20260703_cast-benches.md:13` has casting RTT *alone* at
  8.802 µs p50 (which can't be under a full round-trip). The concept index
  mirrors README per house rule (match the authoritative doc), so the fix is
  to reconcile README ↔ benches.md ↔ the cast-bench report to one current,
  dated number — not to edit the concept in isolation.
- **DOC-PRICELEVEL-24B-STALE** (LOW, docs) — `specs/2/21-orderbook.md:309`
  says `PriceLevel` is 24 B; the code is 32 B (`rsx-book/src/level.rs:20`,
  `const _: assert!(size_of::<PriceLevel>() == 32)`), and `Orderbook::new`
  allocates two arrays of them (active + staging), so ~40 MB of level
  storage for the 617k-slot BTC-PERP example, not the ~15 MB the spec's math
  implies. Concept page fixed; spec still stale.

## Status — 2026-07-08 — userspace-UDP blocked by cast socket coupling

- **CAST-SOCKET-COUPLING-BLOCKS-IOURING** (roadmap, not a defect) — the
  userspace-UDP / io_uring move (matching ME hot-path `recvfrom`/`sendto`;
  gateway/marketdata edges) can't be done in the callers today because
  `CastSender`/`CastReceiver` (`rsx-cast/src/cast.rs`) **own the `UdpSocket`
  and couple framing with `recv`/`send`** — `try_recv_with` does recvfrom +
  parse in one; `send_framed` writes the built `Framed` on the owned socket.
  io_uring must live in the caller (rsx-cast stays runtime-dep-free), so the
  caller needs to own the socket. **Fix (two additive cast APIs, a sanctioned
  frozen-cast extension — needs founder sign-off, not a redesign):** (1) expose
  a built `Framed`'s bytes (`Framed::as_bytes()` or similar) so the caller
  io_uring-sends them; (2) a parse-already-received-bytes entry
  (`CastReceiver::process(&[u8])` / a standalone frame parser) so the caller
  io_uring-recvs then hands bytes to cast for framing/WAL. Single-packet
  request-response gains little from batching — the real lever is SQPOLL — so
  this is a throughput/scaling step, on the roadmap (README §Roadmap), not a
  correctness fix. rsx-cast itself is sound for the current std-UDP path.

## Status — 2026-07-08 — gateway ↔ marketdata alignment (symmetry audit)

Gateway (client-WS in → cast out) and marketdata (cast in → public-WS out) are
two forks of one monoio shape (single reactor, `Rc<RefCell<State>>`,
`ConnectionState.outbound: VecDeque`, `drain_outbound`, per-source
`CastReceiver` + fault/replay). Most divergence is accidental drift; the two
real bugs are **mirror halves of the same missing symmetry** — each crate has
the fix the other lacks. Record-only per triage. (SPSC/tile note: neither uses
rtrb rings — that's Risk; these are the monoio fan-in/fan-out processes.)

- **MD-EGRESS-STALL** (HIGH, latency) — **FIXED 2026-07-08** (`353b347`).
  marketdata's handler blocked on `ws_read_frame` with no egress wake, so a
  listen-only subscriber only got updates when it next sent a frame. Fixed
  with the notify-based egress wake (below) — better than the originally
  proposed bounded-poll port. A later fable analysis found the *gateway's*
  own "500µs" poll was actually ~1–1.5 ms (monoio 0.2.4's timer wheel is
  ms-granular and rounds up), so both gateway and marketdata got the
  event-driven fix: a hand-rolled `!Sync` `EgressWaker` (`egress.rs` in each
  crate), the per-conn handler `select!`s `{readable, egress.wait()}`, and
  every `outbound.push_back` producer signals the waker. Zero extra cores,
  cancel-safe (awaits readiness, not a buffered read). `+TCP_NODELAY` on
  accept (`1ac4efa`). Commits `12de610` (gw) / `353b347` (md) / `1ac4efa`.
- **GW-OUTBOUND-UNBOUNDED** (MED, resource/DoS) — CONFIRMED. `rsx-gateway/src/
  state.rs:144` (`push_to_user`) / `:152` (`broadcast_heartbeat`) `push_back`
  onto `outbound` with no cap; a slow/non-reading authed client accumulates
  unbounded `Arc<str>`. Marketdata bounds the identical structure at
  `state.rs:73` (`max_outbound` → drop + snapshot resync). Adopt that cap +
  `RSX_GW_MAX_OUTBOUND`. (Mirror of MD-EGRESS-STALL, other direction.)
- **WS-FORK-DRIFT** (MED, duplication) — CONFIRMED. `rsx-gateway/src/ws.rs`
  and `rsx-marketdata/src/ws.rs` are forked copies of one WS impl; the gateway
  hardened its reader (mask required, FIN=0 reject, 4 KB cap) and marketdata
  kept the looser copy (mask optional, fragments accepted, **1 MB cap** on an
  unauthenticated public endpoint — MD-FRAME-CAP / MD-WS-UNMASKED). Converge
  the behavior in place (port the guards + 4 KB cap to marketdata); only THEN
  consider hoisting the domain-agnostic core (`WS_MAGIC`, accept loop, frame
  read/write) into a shared module. Auth/jti stays gateway-only.
- **MD-ACCEPT-DROPS-PEER** (LOW, hardening) — CONFIRMED. `rsx-marketdata/src/
  ws.rs:17` types the accept closure `Fn(TcpStream)` and discards `peer`
  (`main.rs:332`); the public endpoint has no per-IP/connection cap and can't
  even see the source IP. Keep the signature symmetric with the gateway
  (`Fn(TcpStream, SocketAddr)`) so a cap becomes possible.
- **MD-PIN-DOC-CONTRADICTION** (LOW, docs) — CONFIRMED. marketdata pinning is
  described three contradictory ways: `ARCHITECTURE.md:174` "no core_affinity",
  README "busy-spin loop", root `CLAUDE.md` "pinned for keep-up"; code pins iff
  `RSX_MD_CORE_ID` set + busy-polls via `sleep(Duration::ZERO)`. Reconcile to
  one story (busy-poll-for-keep-up, optionally pinned).
- **GW/MD-README-ENV-DRIFT** (LOW, docs) — CONFIRMED. Both READMEs document
  env names the code doesn't read (`RSX_GW_LISTEN_ADDR` vs code `RSX_GW_LISTEN`;
  `RSX_GW_HEARTBEAT_INTERVAL_MS` vs `_S`; marketdata README `RSX_MKT_*` vs code
  `RSX_MD_*`). Also `rsx-gateway/ARCHITECTURE.md:133` says "10ms" where code is
  500µs. Fix the env tables on both.
- **GW-HEARTBEAT-DUAL-PATH** (LOW, complexity) — CONFIRMED. Gateway sends
  heartbeats from both the per-connection handler (`handler.rs:88`) and the
  main-loop `broadcast_heartbeat` (`main.rs:477`), which doesn't update
  `last_heartbeat_sent_ns` → ~2× heartbeats. Marketdata does only the main-loop
  path. Collapse the gateway to the simpler shape.
- **DRAIN-REPLAY-4X-DUP** (LOW, duplication — do NOT act) — `drain_replay` is
  byte-identical across gateway/marketdata/matching/risk (4 copies) but builds
  a tokio runtime, so its natural home (rsx-cast) can't take it — rsx-cast is
  FROZEN and never gets a runtime dep. Extraction needs founder sign-off on a
  NEW shared crate. Record only.

## Status — 2026-07-07 — rsx-book compressed-level audit (fable)

**RESOLVED 2026-07-07 — all five below FIXED.** The compressed / mixed-slot
class is fixed independent of zone, and recentering is wired into the live
ME. Commits: `69c81dc` (zone-0 exact 1:1 sizing), `40b3252` (per-side
occupancy + BBA + matching side-predicate + FOK side-guard), `fcfbcdf`
(regression tests — `rsx-book/tests/mixed_slot_test.rs`), `7af7438`
(`recenter_now` eager migration + `rsx-book/tests/recenter_wiring_test.rs`),
`b8e9ef2` (ME `maybe_recenter` wiring + cancel-path BBO). Per bug:
- BOOK-RECENTER-UNWIRED → wired via `maybe_recenter`/`recenter_now`
  (eager, not lazy — see BOOK-MIGRATION-PARTIAL-BOOK below).
- BOOK-MIXED-SIDE-SELF-TRADE → `match_at_level` matches only opposite-side
  crossing makers; `can_fill_fully` compressed walk is side-guarded.
- BOOK-STALE-OCC-ME-CRASH → `PriceLevel.{bid,ask}_count`; per-side
  occupancy set/clear; `emit_bbo` tripwire.
- BOOK-FOK-CLAMP-DIVERGENCE → zone 0 sized `2*ceil(pct_5/tick)` (no
  clamp aliasing) + side-correct FOK feasibility.
- BOOK-STALE-BBA-WRONGFUL-POSTONLY → cancel + match loop refresh best via
  scan + true best price in the slot (partial-empty aware).
- BOOK-MIGRATION-PARTIAL-BOOK → moot for the live ME (it uses the eager
  `recenter_now`, never a partially-migrated book); the incremental
  `trigger_recenter` + `migrate_batch` API still has the partial-book
  property but has no live caller.

Original triage (kept for the record):

**OPEN (triage) — one PRIMARY cause, four downstream symptoms.**

- **BOOK-RECENTER-UNWIRED** (CRITICAL, integration — the real defect) — the
  compression map is only 1:1 within zone 0 (±5% of mid); past that it merges
  ticks, and a merged slot can hold **both order sides and multiple raw
  prices**. The design keeps the touch inside zone 0 by *recentering* the mid
  as the market drifts — and that state machine is fully built AND unit-tested
  (`should_recenter` → `trigger_recenter` → `resolve_level`/`advance_frontier_to`
  → `migrate_batch` → `complete_migration`; see `migration.rs`,
  `tests/{recentering,migration,distribution}_test.rs`). **But it is never
  wired into the live ME.** `should_recenter`/`trigger_recenter`/`resolve_level`
  have ZERO callers in `src/` (tests + benches only); `rsx-matching/src/main.rs`
  calls only `migrate_batch(100)` on idle, which `return`s immediately because
  `state` is never `Migrating` (nothing calls `trigger_recenter`). So the mid
  is pinned at the seed forever; a stale seed or a >5% move parks the touch in a
  coarsened zone, and the four symptoms below become reachable. With the touch
  in zone 0 (1:1) none of them can fire. Fix: wire the trigger into the ME loop
  (plan: `.ship/plan-book-recenter-wiring.md`).

The four symptoms below are **only reachable once the touch leaves zone 0**
(i.e. only because BOOK-RECENTER-UNWIRED lets the mid go stale). Mechanism:
`match_at_level`, per-side occupancy maintenance, and the BBA px cache each
assume one side / one price per level. Existing smooshed-tick tests
(`tests/matching_test.rs:447-520`) only used same-side bands, so the class was
uncovered. Verified with a runnable repro (compiles against real `rsx-book`;
#2 panics live). NOT fixed — recording only.

- **BOOK-MIXED-SIDE-SELF-TRADE** (CRITICAL, correctness) — a taker fills a
  resting order of the **same side** (position corruption). `matching.rs:287-300`
  compares only maker *price*, never maker *side*; `book.rs:154-239` links
  opposite sides into one level. Repro (mid=50_000, tick=1): GTC BUY 47_491/10,
  GTC SELL 47_495/10 (rests at the same tick 5_499), GTC SELL 47_490/20 → the
  sell taker fills the resting SELL @47_495. `update_positions_on_fill` then
  *increases* the seller's net_qty. Post-only can also take liquidity via this
  path. Violates fill semantics + invariant #4.
- **BOOK-STALE-OCC-ME-CRASH** (CRITICAL, correctness/liveness) — a stale
  side-occupancy bit points `best_*_tick` at an empty level → `emit_bbo`
  dereferences `head == NONE` → **ME panics** (`slab.rs:92` index
  4294967295), and WAL replay re-runs the sequence → **crash loop**.
  `book.rs:204-209` sets side-occupancy only on empty→non-empty (2nd
  opposite-side order never sets its bit); `book.rs:264-273` clears only the
  departing side on →empty; `matching.rs:246-259` then hits the empty head.
  Repro: the 3 orders above, then GTC BUY 47_000/10, GTC SELL 46_900/10 →
  panic inside `process_new_order`. Even without the panic the stale bit
  shadows real bids → persistently crossed book. Violates invariants #6/#7.
  **This is the live-verified one (repro test panics).**
- **BOOK-FOK-CLAMP-DIVERGENCE** (HIGH, correctness) — FOK feasibility
  (`can_fill_fully`, `matching.rs:397-405`) treats a zone-0 level as
  single-price, but `compression.rs:56,113` (floor-division `z0` + the
  `local_offset.min(half-1)` clamp) aliases two distinct tick-aligned prices
  into one zone-0 slot whenever `(mid*5/100) % tick_size != 0`. Repro
  (mid=50_001, tick=3): sells @52_497 and @52_500 alias to one slot; FOK BUY
  52_497/20 passes feasibility, matches only 10 → debug panic
  `matching.rs:191`; **release compiles the assert out → emits a Fill AND
  OrderFailed for the same taker with positions already mutated**. Violates
  invariant #2 (exactly-one completion). No config validation prevents the
  precondition.
- **BOOK-STALE-BBA-WRONGFUL-POSTONLY** (MED, correctness) — `best_bid_px`/
  `best_ask_px` go stale when a smooshed level *partially* empties (cancel or
  partial fill leaves survivors at a different price); `book.rs:299-308` /
  `matching.rs:136-158` rescan BBA only when the level fully empties. Repro:
  sells @52_501 and @52_509 (same tick), cancel the @52_501 → `best_ask_px`
  still 52_501; post-only BUY 52_505 → wrongly CANCEL_POST_ONLY though it
  crosses nothing. `price_at_tick` also returns the head's price, not the
  band's best.

**PLAUSIBLE (code-traced, not run; lower exposure):**
- **BOOK-MIGRATION-PARTIAL-BOOK** (traced) — matching during a recenter
  ignores unmigrated levels (`migration.rs:22-80`; `resolve_level` has no
  caller); missed fills / later crossed book. Dormant: `trigger_recenter`
  has no production caller today.
- **BOOK-MODIFY-STALE-HANDLE** (traced) — `modify_order_price`
  (`book.rs:354-379`) reads slot data before any `is_active`/identity check;
  no checked variant. No production caller (tests only) — latent API hazard.
- **BOOK-ORDER-COUNT-U16-WRAP** (traced) — `UserState.order_count` is u16
  unchecked (`book.rs:236`); ME slab cap is 65_536, so a user holding 65_536
  orders wraps to 0 → `is_idle()` true → `try_reclaim` frees a live user.
- **BOOK-SNAPSHOT-FREELIST-PERMUTE** (traced) — `snapshot.rs:179-183`
  rebuilds the freelist descending, not the live LIFO order; handle
  assignment diverges post-restore. Determinism hazard only if a consumer
  keys on handles across restore.

## Status — 2026-05-30

**OPEN (triage):**
- **ENV-TEST-PARALLEL-RACE** (LOW, test-isolation) — **Status: FIXED
  2026-07-07.** The two env_* tests in each `me_cast_addrs_test.rs`
  (marketdata + risk) mutate process-global `RSX_ME_CAST_ADDR(S)` and raced
  when run on parallel threads within a binary. Fixed with a file-local
  `static ENV_LOCK: Mutex<()>` (poison-tolerant `.unwrap_or_else(into_inner)`)
  guarding the two env tests in each file — no new dep. Verified: 10/10
  green over repeated default-parallel runs.
- **CAST-BIG-GAP-ESCALATION** (LOW, design) — a NAK gap that stays *under* the
  2048 reorder ring recovers seq-by-seq (each fill retransmit is a WAL scan);
  only ring overflow escalates to the TCP-replay cold path. A gap far beyond
  WAL retention could proactively escalate to replication instead of grinding.
  Characterized 2026-07-07 by the landed `loss_degradation` + `outage_recovery`
  benches: the overflow→replay path works (52 k-record gap replays in ~1 s),
  and the live NAK path reliably delivers through ~30% loss; the grind only
  bites a slow-growing sub-ring gap. Enhancement, not a bug. (The 2026-07-06
  "run_once hung" finding was a HARNESS bug — the callback didn't stop on
  `RECORD_CAUGHT_UP` so it tailed live forever, plus a WAL firehose; fixed in
  the landed outage bench, 25c9aa2 / 50c78f9. Not a cast defect.)
- **TIME-NS-HOTPATH-AUDIT** (LOW, perf) — flagged 2026-07-06 during the
  rsx-cast review. `time_ns()` (clock_gettime via vDSO, ~15-30 ns) is called
  per-record in a few spots (replication-server CaughtUp/record stamping, WAL
  timestamps). Audit call frequency; if any is a true per-record hot path,
  stamp once per batch or use a coarser/cached clock. Not correctness — record
  + reconsider later.
- **REPLAY-RESCAN-PER-CONNECTION** (MED, perf) — flagged 2026-07-06. The
  replication server's `oldest_and_highest_seq` full-scans the *active* WAL
  file end-to-end (≤64 MB) on **every** replay connection to validate the
  request range. Fine for occasional bootstraps, but under a **reconnect
  storm** (a consumer fault-looping) it re-scans repeatedly — disk replay
  becomes I/O-bound / near-impractical. Server is a stateless dir-reader by
  design, so options that keep that: (1) incremental tail-scan (cache
  highest + active-file offset, scan only new bytes); (2) producer-published
  highest-tip file (~10 ms stale is fine for the refusal check); (3) shared
  atomic if server+writer co-located (most coupling). Leave for now.
  Confirmed 2026-07-07 by the `outage_recovery` bench: per-cycle recovery of a
  constant ~7.5 k-record gap drifts 262 → 786 ms across six cycles purely
  because the active WAL grows and each connection re-scans it.
- **BENCH-NO-TIMEOUT-GATE** (FIXED 2026-07-07) — flagged 2026-07-07;
  nothing time-bounded bench execution (no `timeout` in
  `scripts/bench-gate.sh` or the Makefile bench targets, Criterion has no
  per-bench deadline), so a hanging bench (CAST-RTT-BENCH-DEADLOCKS-ON-LOSS
  hung 50 min, then 240 s again 2026-07-07) read as "slow", not FAILED.
  Fixed: `scripts/bench-gate.sh` wraps its `cargo bench --workspace` in
  `timeout 600`; `make perf` wraps its `cargo bench` the same way. A hang now
  exits 124 -> gate fails.
- **CAST-RTT-BENCH-DEADLOCKS-ON-LOSS** (FIXED 2026-07-07) — flagged 2026-07-06.
  TWO stacked hangs, both fixed in the bench: (1) deterministic — the
  send<T> removal (bb6c1a0) moved seq ownership to the caller, and the
  bench's counter lived inside Criterion's |b| closure, which re-invokes
  per phase → counter reset → receiver dedup-drops everything → deadlock
  at warmup→collection (100% repro). Hoisted the counter out. (2) the
  original probabilistic loss-deadlock below — echo-wait now pumps
  tick+recv_control on a coarse cadence. Verified 3 clean runs
  (~9.4-9.7 µs). Original analysis:
  `cast_rtt_bench`'s A-side echo-wait is `loop { try_recv; spin_loop }` with NO
  in-loop NAK recovery: A sends seq=N, spins until B echoes. If that datagram
  (or its echo) drops on loopback, B never echoes, A never advances `iter`, so A
  never reaches its `iter & 0x3FF == 0` recv_control/tick — B's NAK goes
  unanswered → permanent deadlock (observed: 50 min stuck in a 3 s warmup).
  Pre-existing (predates the send<T>→send_framed migration, which only changed
  the seq source, values identical); earlier runs got lucky on a loss-free
  window. Fix: recv_control()/tick() *inside* the echo-wait spin (with a bounded
  spin count) so a lost frame retransmits, or add a per-iter deadline. Until
  then the casting-RTT row is measured opportunistically (~9.6 µs, 2026-07-06).
- **STARTUP-ORDERING-FRAGILITY** (MED, ops) — "can't start the system via the
  playground" traced to a chain of order-dependencies, none self-healing:
  (1) **Postgres must be up first** — if the `rsx-postgres` container is stopped/
  gone, risk-0 crash-loops "error connecting to server" and nothing comes up;
  the playground still reports "started 6 processes" (misleading). Mitigated:
  `docker update --restart unless-stopped rsx-postgres` so it survives.
  (2) **Playground manages the cluster in-memory** — restarting the playground
  server orphans the processes it spawned and they die; the new instance starts
  with an empty process table. (3) **Playground marketdata subscriber
  circuit-breaks** if marketdata isn't live when the playground starts ("md
  subscriber circuit open: N failures; pausing fan-out") and does NOT
  auto-recover → `_book_snap` stays empty → orders can't price → rest/reject.
  (4) Fresh PG needs `seed_accounts` (runs at playground startup) + a risk
  restart to load them. (5) `/api/risk/users/N` mislabels an empty-positions
  result as "no postgres connection". Correct bring-up order: PG → playground
  server → cluster (start-all) → maker → deposits. Fix: a supervised start
  sequence (or make PG a compose service + the playground reconnect its md
  subscriber + not lose the process table on restart).

- **HEALTH-PORT-BAND-OVERLAP-AT-SID-10** (FIXED 2026-07-07) — flagged by the
  deploy authoring (2026-07-06). The `start`/spawn-plan health-port band math
  (`base + sid` / `base + shard`) collided at sid≥10: PENGU sid=10 → health
  `9810` overlapped the risk health base `9810`. Fix: widened the per-tier
  stride from 10 to 100 ports (`BASE_ME_HEALTH=9800`, `BASE_RISK_HEALTH=9900`,
  `BASE_GW_HEALTH=10000`, `BASE_MARK_HEALTH=10100`, `BASE_MD_HEALTH=10200`,
  `BASE_RECORDER_HEALTH=10300` in `./start`) — no `base+sid`/`base+shard`
  offset below 100 can now cross into another tier's band. Verified by
  simulating sid/shard 0..15 across all tiers: zero cross-tier collisions
  (see RISK-REPLICA-HEALTH-INTRATIER-OVERLAP below for a pre-existing,
  separate intra-tier collision this did NOT touch).
- **RISK-ENV-VARS-SET-NOT-READ** (FIXED 2026-07-07) — `RSX_RISK_RESET_FROZEN_
  ON_START` and `RSX_RISK_IS_REPLICA` were set by the `start` spawn plan (and
  `RSX_RISK_IS_REPLICA` also by `rsx-playground/tests/acceptance_test.py`)
  but read nowhere in `rsx-risk/src`. Fix: dropped both from `./start` and the
  acceptance test's risk env dict; also scrubbed `rsx-risk/README.md`'s
  now-false claim that `RSX_RISK_IS_REPLICA=true` selects replica mode
  (role is decided by advisory-lock acquisition at startup, per
  `specs/2/28-risk.md`'s own description of `main()`'s state machine).
- **RISK-REPLICA-HEALTH-INTRATIER-OVERLAP** (LOW, config, newly noticed
  2026-07-07 while verifying the fix above) — inside the risk health band
  itself, the replica offset formula (`BASE_RISK_HEALTH + 10 + shard*2 + r`)
  can collide with a *primary* shard's health port (`BASE_RISK_HEALTH +
  shard`) once `risk_shards` gets into double digits, e.g. shard=10 primary
  == shard=0,r=0 replica (`base+10`). Pre-existing (not introduced by the
  100-port-stride fix above, just carried the same shape forward); latent
  because current scenarios use ≤3-4 risk shards. Not fixed here — separate
  from the requested sid≥10 cross-tier bug, recorded for later triage.

- **MATCHING-BENCH-ORDERTYPE-FIXTURE** (LOW, bench) — **Status: FIXED
  2026-07-04 (`da9a2b4`).** `match_by_type` (`ioc_full`, `gtc_full_cross`,
  `sweep_10_levels`) measured 32–120 µs where the match algo is ~30-60 ns. The
  original triage guessed "`iter_batched` fixture alloc/drop bleed" — WRONG.
  The real cause: `taker_fill` clears the touch level on every call (unlike
  `match_ioc_vs_1k_asks`'s replenish-before-clear pattern), and `scan_next_bid`/
  `scan_next_ask` were an O(compression-slots) linear scan that only ran when
  a level actually emptied — so any bench whose op clears a level paid the
  full ~100k-slot scan, while `post_only_reject` (crosses nothing, never
  clears a level, 6 ns on the SAME depth-10k fixture) proved the fixture
  itself was cheap. Fixed by `da9a2b4`: hierarchical occupancy bitmap
  (`rsx-book/src/occupancy.rs`), O(depth=3) find-next/find-prev. Confirmed
  2026-07-04 re-run: `match_ioc_vs_1k_asks` 4.37µs→145ns, `match_by_type/
  ioc_full` ~80µs→79ns, `match_by_type/sweep_10_levels` ~1ms→700ns. See
  `reports/20260704_book-bench.md` "post-scan-fix" section. `match_by_depth`
  (~60ns flat, never clears the touch level) was correctly unaffected all
  along.
- **FOK-AVAILABLE-LIQUIDITY-ON-SCAN** (MED, bench) — **Status: FIXED
  2026-07-04.** `match_by_type/fok_full` was ~296 µs after the occupancy-
  bitmap fix (`da9a2b4`), unlike every other order type (60-145 ns). Cause:
  the old `available_liquidity` was a SEPARATE O(N-resting-orders) full-book
  scan run on FOK's hot path — it walked every one of the ~100k active
  levels and every order on each, summing crossable qty BEFORE matching.
  Fix: no new structure. FOK is just "try to match it, take it or don't", so
  `can_fill_fully` (`rsx-book/src/matching.rs`) now walks only the *crossing*
  levels in price order — the same traversal a match performs, via the
  book's existing best-level index — summing each level's already-maintained
  `total_qty` and stopping the instant the running total reaches the order
  size. O(levels crossed, early-exit) instead of O(slots + orders); a whole
  level shares one price so `total_qty` counts it exactly (no per-order
  walk). Pinned to brute-force by `tests/fok_liquidity_test.rs` (3000 FOK
  probes over multi-zone random flow). Post-fix `fok_full`: ~118 ns
  (was ~296 µs, −99.95% per Criterion) — now in the same band as every
  other order type. (Number from a lightly-contended box; clean re-run
  pending, but the 300x magnitude is unambiguous.)
- **RECORDER-ARCHIVE-DEV-DISK** (MED, *reframed 2026-07-04*) — the recorder
  archives the FULL ME WAL stream (every order/fill/BBO/done record, verbatim)
  to `tmp/wal/archive/<sid>/<sid>_<date>.wal` as the permanent system-of-record.
  **Unbounded is BY DESIGN** — this is the exchange's audit trail + replay-from-
  genesis tier ("ARCHIVE handles long-term durability", hot tier keeps only 4h).
  Do NOT add retention/GC that deletes records — that destroys the point.
  The actual defect is PLACEMENT: it writes to the local dev root (237 GB) with
  no offload, so continuous maker quoting grew it to 59 GB and ENOSPC'd the box
  (killed a subagent + failed cluster stop/start) twice this session. Fix is
  storage, not deletion: put the archive on a separate/dedicated volume, offload
  to object storage (S3/GCS) with local pruning of already-offloaded segments,
  and/or a DEV-ONLY guard (cap or recorder-off in the playground) so the dev box
  can't fill. Currently mitigated by keeping the recorder stopped in dev.
  Cleared manually (find -delete + kill recorder to release the fd) → 85 G free.
- **BENCH-MOLD-SOUP-UNPINNED** (FIXED 2026-07-07) — flagged as `compare_moldudp64`
  + `compare_soupbintcp` never pinning their threads while casting/raw-UDP/
  KCP/Aeron pin client->core2/echo->core3. Verified already fixed by
  `cad490d`/`aa6ff17`/`e48a36c` (both benches call the same `pick_cores()` +
  `core_affinity::set_for_current` pattern as `compare_all`) — no code change
  needed, entry closed.

- **BENCH-QUINN-ACCEPT-BI** (LOW, *unmasked by the KCP fix*) — with KCP no
  longer aborting the run, `compare_all` now panics at
  `benches/compare_all.rs:356`: `srv_conn.accept_bi().await.unwrap()`. QUIC
  opens a bidirectional stream lazily — the client's `open_bi()` sends nothing
  on the wire until the first `write`, so the server's `accept_bi()` never sees
  the stream (resolves to a connection error / hang). The Quinn row was masked
  by the earlier KCP abort, so this has likely been broken since KCP regressed;
  the README "~37 µs" is last-measured 2026-05-24, not reproducible now. Fix:
  have the client write one priming byte after `open_bi()` before the server
  `accept_bi()`, or restructure the stream handshake. Bench-only. Flagged, not
  patched (separate from the KCP one-liner; QUIC stream-lifecycle surgery).
- **GATEWAY-LATENCY** (MED, *readiness fix landed*) — the cast-recv yield-spin is
  gone: the gateway now awaits io_uring readiness on the CastReceiver fd
  (`946b71d` + `7454187` exposes the fd), event-driven not polled. Earlier
  500µs conn-side egress poll (`5a578d3`) may remain; re-measure with
  `ws_order_latency` on a QUIET box to confirm the win + whether the conn poll
  still shows. Detail below.

**FIXED 2026-07-04** (detail sections below): ME-FAULTED-NO-REPLAY-ADDR,
IOC-NOT-HONORED.

**NEW 2026-07-04** (sonnet bug-hunt, all verified genuine — detail at end):
COMPRESSION-ZONE-TICK-UNIT (HIGH, latent), WAL-ROTATE-PREWRITE-MISLABEL (HIGH),
CLI-PTR-READ-UNALIGNED-UB (HIGH), CAST-SEND-RING-TOO-SMALL (MED),
GW-CANCEL-NO-RATELIMIT (MED), GW-CANCEL-NOT-USER-SCOPED (MED),
RISK-DEDUCT-FEE-UNCHECKED (LOW), MARK-PARSE-NEG-ZERO (LOW).

**NEW 2026-07-04** (T4 `.ship/33-TUI-SPEED-TESTS`, `rsx-tui/tests/
e2e_guarantees.rs` fixture debugging — detail at end): VERIFY-WAL-FILLS-
ALWAYS-ZERO (LOW), DEMO-TRADE-SUBMIT-ORDER-404 (MED).

**DEFERRED — book session** (founder: "solve once we're dealing with book"):
BOOK-FAR-PRICE-BUCKETING (`[D]` by-design, no action). Detail below.
**FIXED 2026-07-04** (book session): FOK-RESTS-IN-COMPRESSED-ZONES (new,
HIGH), BOOK-SLAB-FREE-UNGUARDED, BOOK-STALE-HANDLE-REUSE,
ME-REDUCEONLY-IOC-FILLEDQTY — detail below. (BOOK-BBO-COMPRESSED-INDEX +
BOOK-SCAN-NEXT-BID-OFFBY were fixed 2026-07-03 — see git/CHANGELOG.)

**BY-DESIGN (no action):** RISK-FUNDING-CROSS-SHARD (global zero-sum not
guaranteed across shards; demo is single-shard), GW-SINGLE-SHARD-NO-ROUTING
(one risk sender, no `user_id % shard_count`; demo limit), ME-REPLAY-SKIPS-
DOWNSTREAM (each consumer recovers independently via its own replay).

---

## ME-FAULTED-NO-REPLAY-ADDR — ME panicked on a dropped-packet order gap (MED)

**Status: FIXED 2026-07-04.** Resolved by the founder-blessed fault model:
the risk→ME **order** stream is drop-safe, so on FAULTED the ME now **skips the
gap and resumes live** rather than replay-or-panic. Rationale: a dropped
pre-ack order is re-sent by the client (no-ack-within-timeout,
`specs/2/49-webproto.md`) and deduped on the ME's WAL (`RECORD_ORDER_ACCEPTED`)
= exactly-once; and the ME re-sequences on output (its own WAL seq), so an
inbound gap is never an output gap (risk/recorder/marketdata see a contiguous
ME stream). The FAULTED handler (`rsx-matching/src/main.rs`) now counts skipped
seqs into `gauges.drops`, WARNs the gap range, and calls
`reset_after_replay(gap_end_inclusive)`. The dead consumer-side replay path
(`RSX_ME_REPLICATION_ADDR` + `drain_dxs_replay_into_book` + `apply_replayed_
record` + `replay_after_fault_test`) was removed (306-line `replay.rs` +
its test). **Still in place (different, still-required):** the ME's WAL
replication *server* (`RSX_ME_REPLICATION_BIND_ADDR`) that RISK pulls for
**fill** recovery, and ME cold-start replay from its own local WAL. Found
2026-05-30 under parallel-load e2e (single dropped UDP packet → FAULTED →
panic because `RSX_ME_REPLICATION_ADDR` was unset). Note: risk/marketdata/
gateway consumers still panic on FAULTED — those are separate streams with
their own recovery needs, out of scope here.

## IOC-NOT-HONORED — cancelled IOC surfaced to client as "resting" (MED)

**Status: FIXED 2026-07-04.** Root cause was NOT tif propagation (the original
triage guessed that and was wrong). Verified on the live cluster with a runtime
trace: `tif=1` reaches `risk_in` AND `me_in` intact, and the matching engine
**correctly cancels** the residual IOC (`rsx-book/src/matching.rs` residual
branch fires → `OrderDone { reason: REASON_CANCELLED }`). The order does NOT
actually rest in the book. The bug was a code-space collision: the ME wrote
`OrderDoneRecord.final_status = reason` (raw matching reason, `REASON_CANCELLED
= 1`), but per `specs/2/49-webproto.md` `final_status` is a webproto U-frame
status where **1 = RESTING, 2 = CANCELLED**. So a cancelled IOC's `final_status
= 1` was forwarded by the gateway as status 1 → the client saw "resting".
`REASON_FILLED = 0` happened to equal webproto FILLED(0), so fills looked fine
and hid the collision. **Fix:** `rsx-matching/src/wal_integration.rs` now
translates the matching reason → webproto status (`done_final_status`:
CANCELLED→2, FILLED→0) at both OrderDone-build sites; the gateway
`route_order_done` mapping was already spec-correct. Regression:
`rsx-matching` `tests/wal_integration_test.rs::
ioc_cancel_final_status_is_webproto_cancelled`. Live-verified: an empty-book /
non-crossing IOC now reports "cancelled" (fills still "filled", resting GTC
still "resting").

## GATEWAY-LATENCY — casting-recv poll-loop starvation dominates e2e (HIGH, mitigated)

**Status: MITIGATED (full fix deferred).** Single-order stage trace (live
cluster): the response left Risk by ~571µs (`me_out`) but the gateway didn't
receive it (`gateway_cast_recv`) until much later — the response sat in the
gateway UDP socket buffer waiting for the casting-recv poll loop to get a turn
on the shared monoio reactor (WS accept + per-conn handlers + casting-recv all
on one reactor). The egress-drain poll was tightened 10ms→500µs (`5a578d3` +
handler), which dropped WS single-stream p50 from 11.5ms to 2.25ms
(`reports/20260530_e2e-ws-latency.md`). **Remaining fix (deferred):** tile-split
the casting-recv response path to a dedicated pinned busy-spin thread (off the
reactor) → SPSC ring → WS writer tasks (same pattern as Risk/ME). Biggest single
e2e win; deferred per founder ("don't split now").

## Deferred book-session bugs (detail)

Founder: solve these when we next work the book. All verified against source;
`[V]` real, `[?]` needs one more check, `[D]` by-design/known-limitation.

- **FOK-RESTS-IN-COMPRESSED-ZONES (HIGH, latent correctness).** Status: FIXED
  2026-07-04. `can_fill_fully` (`matching.rs`) summed a whole level's
  `total_qty` but tested only the HEAD order's price against the taker limit. In
  a compressed zone (≥1: 10/100/1000 ticks/slot, plus the zone-4 catch-all) a
  level holds orders at DISTINCT raw prices, so it over-counted crossable
  liquidity → FOK feasibility passed → `match_at_level` skipped the non-crossing
  makers → residual > 0 → and the residual branch only cancelled IOC, so the FOK
  fell through to `insert_resting` and RESTED (all-or-nothing violation, with a
  partial fill). Two-part fix: (a) `can_fill_fully` now walks the level's orders
  and sums only qty whose ACTUAL raw price crosses the limit whenever the slot
  is in a compressed zone (`t >= zone_slots[0]`); zone 0 keeps the O(1)
  `total_qty` shortcut (single price per slot, the near-BBO happy path), and
  early-exit is preserved (stop once a whole band sits beyond the limit). (b)
  the residual branch now rejects FOK (`OrderFailed(FAIL_FOK)`, book untouched)
  instead of resting — defense in depth behind the now-exact pre-check
  (`debug_assert!(false)` guards the can't-happen residual). Regression:
  `tests/fok_liquidity_test.rs` (`fok_compressed_zone_insufficient_true_
  liquidity_rejected`, `_tick50_insufficient_rejected`, `_sufficient_liquidity_
  fills`), each fails on the old `matching.rs`.
- **BOOK-SLAB-FREE-UNGUARDED `[V]` (hardening).** Status: FIXED 2026-07-04.
  `slab.free` now `debug_assert`s `idx < bump_next` (never-allocated) and
  `!is_free(idx)` (already on the freelist) — a double-free / freelist cycle is
  caught in debug rather than aliasing a slot handed out twice. O(free)
  is-free walk stays behind `debug_assert` (off in release; the ME bounds open
  orders upstream). Regression: `tests/slab_test.rs::slab_double_free_panics`,
  `slab_free_never_allocated_panics`.
- **BOOK-STALE-HANDLE-REUSE `[?]`.** Status: FIXED 2026-07-04 (defensive).
  Full generational handles would ripple the `u32` handle meaning across
  matching/events/index (too invasive), so instead: added
  `Orderbook::cancel_order_checked(handle, user_id, order_id_hi, order_id_lo)`
  which re-checks the slot's identity before cancelling (returns false, book
  untouched, on a reused/inactive slot), and documented on `cancel_order` the
  exact cross-crate invariant rsx-matching must uphold (verify identity, or use
  the checked path). rsx-matching's user-cancel path already does this drift
  check inline (`main.rs:1002-1017`); the WAL-replay path trusts its own
  `order_index`. Regression: `tests/book_test.rs::
  cancel_order_checked_rejects_stale_handle`.
- **ME-REDUCEONLY-IOC-FILLEDQTY `[?]`.** Status: FIXED 2026-07-04.
  `matching.rs` computed `filled = order.qty - order.remaining_qty`, counting
  the reduce-only clamp (remaining clamped down to the position) as execution —
  an empty-book reduce-only IOC with qty > position reported `filled = qty -
  position` with zero real fills. Fix: capture `fillable = order.remaining_qty`
  AFTER the clamp (before matching) and measure fills against it at every
  terminal site (IOC residual, FILLED). Regression:
  `tests/matching_test.rs::reduce_only_ioc_empty_book_reports_zero_filled`
  (old code reported filled=70, fixed reports 0).
- **BOOK-FAR-PRICE-BUCKETING `[D]`.** `compression.rs:48,118` — compression
  buckets far prices (10/100/1000 ticks per slot), so distinct prices share a
  level → price-time priority is coarse far from mid. Intentional compressed-book
  tradeoff; logged as a known design risk, not a defect.

### MIGRATE-SKIPS-NEW-MID-LEVEL — order resting at new_mid orphaned on recenter (HIGH)
**Status: FIXED 2026-07-04.** `trigger_recenter` now migrates the `new_mid`
level once, up front (no-op if empty, within-frontier so never migrated twice),
so an order resting exactly at new_mid survives recenter. Covers both the lazy
(`advance_frontier_to`) and eager (`migrate_batch`) paths — new_mid is empty in
`old_levels` by the time either runs. Regression: the adversarial tick-size test
now recenters ONTO a resting level (`new_mid = mid + 505*tick`) and asserts slab
no-leak. Found by the distribution tests. Original triage below.

**[Original — OPEN]** `migration.rs` —
`trigger_recenter` sets `bid_frontier = ask_frontier = new_mid`, then both
`migrate_batch` and `advance_frontier_to` (lazy path) step the frontier AWAY
from `new_mid` BEFORE migrating (bid: `bid_frontier -= tick` then migrate; ask:
`ask_frontier += tick` then migrate). So the OLD level covering exactly
`new_mid` is never visited — any order resting at `new_mid` is left in
`old_levels` and dropped when `complete_migration` clears them. Loss is silent:
one live order vanishes → violates invariant #8 (slab no-leak: allocated != free
+ active) and invariant #4 (position = sum of fills) downstream. Reproduced for
tick_size ∈ {1,10,50}: rest a fat book, recenter to a `new_mid` that coincides
with a resting level, migrate fully → exactly one order missing (len 800, free
40, active 759). Not triggered when `new_mid` falls between levels (the common
case), which is why it stayed latent. Fix: migrate the `new_mid` level once at
`trigger_recenter` (or seed the frontier so the first step includes `new_mid` —
e.g. migrate before decrement/increment). The distribution recenter test
(`rsx-book/tests/distribution_test.rs`) sidesteps it by recentering off a level;
remove that workaround once fixed.

## Dashboard stability + RSX process flapping (2026-05-31, task 29-#12)
- Playground dashboard runs as a SINGLE uvicorn worker (server.py:8127,
  workers=1 reload=False) with no self-watchdog. Any kill = full outage
  until manual `./playground start`. NOTE: several apparent "crashes" this
  session were self-inflicted (`pkill -f server.py` matched the running
  shell; `fuser -k 49171/tcp` deliberate) — the server is more stable than
  it appeared; the real gap is no supervisor.
- RSX processes flap (e.g. 4/7 running): auto-restart supervisor with a
  circuit-breaker opening after 5 crashes -> `blocked` (server.py:344-357).
  Root crash causes: Postgres down (10.0.2.1:5432 unreachable),
  marketdata rcvbuf -> FAULTED, maker-induced cast FAULTED (per MEMORY).
- FIX options (needs decision; edit blocked while overview agent owns
  server.py): (a) run dashboard under systemd/pm2 or a small watchdog;
  (b) bring Postgres up so PG-dependent processes stop crashing;
  (c) `make tune-host` (rmem_max) before enabling the auto-maker to avoid
  the rcvbuf FAULTED loop.

## Control Stop/Start buttons don't work (2026-05-31, task 29)
- Owner reports per-process Stop/Start (control grid + faults page) do
  nothing. Suspect: buttons post ./api/processes/{name}/{action} WITHOUT
  an x-confirm header (only the walkthrough all/start at pages.py:663 sends
  hx-headers x-confirm), while the all/* endpoints require check_confirm.
  Verify the {name}/{action} handler's confirm/run_id gate and either send
  the header from the buttons or drop the gate for single-process actions.
  Must be covered by the audit + Playwright play-tests.

  CORRECTION: the {name}/{action} handler (server.py:3942) needs only
  loopback (no confirm gate), so it's NOT a header issue. Likely cause:
  (a) the raw-PID stop fallback (3958-3972, hit when name not in `managed`
  after a dashboard restart drops the in-memory dict) SIGTERMs but does NOT
  mark intentional/update _restart_state, so the auto-restart watcher
  revives the process; and/or (b) no visible feedback. Fix: route single-
  process stop through the intentional-flag path (like stop_process) even
  on the raw-PID branch, and surface the result. Confirm by clicking Stop
  and watching whether the process reappears.

## Sonnet bug-hunt findings — 2026-07-04 (all verified genuine)

Four read-only sonnet hunters over the whole tree; every finding below was
re-verified against source by the main agent. Recorded, not fixed (bug-triage
protocol — awaiting prioritization).

### COMPRESSION-ZONE-TICK-UNIT — zones mis-sized for tick_size != 1 (HIGH, latent)
**Status: FIXED 2026-07-04 (`9089e50`).** `CompressionMap::new` now stores zone
thresholds as RAW-price distances (matching `price_to_index`'s `price -
mid_price`), and converts to slot counts by dividing by `compression_ticks *
tick_size` — so a 5% move lands in the right zone for any tick_size, not the
2-slot catch-all. Verified: `tick50_five_pct_lands_in_zone0_not_catchall`,
`tick50_zone0_is_one_tick_per_slot`, `tick_size_stored_and_thresholds_raw`,
`adversarial_tick_sizes_matching_and_recenter` (tick 1/10/50 through real
matching + recenter). `migration.rs::should_recenter` checked too. Original
triage below.

**[Original triage — OPEN]** `rsx-book/src/compression.rs:29-45` computes zone thresholds
in TICK units (`pct_5 = mid*5/(100*tick_size)`, comment line 24 "…/ tick_size
ticks"), but `price_to_index` (`compression.rs:84-85`) compares them against a
RAW-price distance (`distance = price - mid_price`, never divided by
tick_size). For `tick_size=1` they coincide (every test/bench uses tick_size=1,
masking it). For `tick_size != 1` (BTC=50, ETH=10 in the symbol config) the
units diverge: a price only 5% from mid lands in the 2-slot zone-4 catch-all →
most of the book collapses into shared price levels → price-time priority
broken, wrong fills. Same mismatch reachable via `migration.rs:16-22`
(`should_recenter`) and recentering. Fix: divide `distance` by `tick_size` (or
store raw-price thresholds). Latent because the demo trades PENGU (tick=1).

### WAL-ROTATE-PREWRITE-MISLABEL — boundary records unreachable for NAK/replay (HIGH)
**Status: OPEN (rsx-cast bugfix candidate).** `rsx-cast/src/wal.rs:220-224` the
pre-write rotation fires before `write_all`, but `rotate()` (line 252-256) names
the old file with `self.last_seq`, which `append_framed` (line 197) already
advanced to include the still-buffered (unwritten) records. So the old file is
labeled `[first_seq..last_seq]` while physically holding only up to the
previously-flushed seq; the buffered records then go to the NEW file whose
`first_seq` is set to `next_seq` (line 282) — above them. Those boundary records
become unreachable via `read_record_at_seq` (NAK retransmit) and
`open_from_seq` (TCP replication catch-up) → both silently return nothing for a
record that exists. Pre-write is the PRIMARY rotation path (same threshold as
post-write, fires first); the existing `write_rotate_read_across_files` test only
hits the correct post-write path (file_size==0 on its single flush). Fix: label
rotate with the last seq actually written to that file, not `self.last_seq`.

### CAST-SEND-RING-TOO-SMALL — NAK fast-path dead for hot 128B records (MED)
**Status: OPEN (rsx-cast bugfix candidate).** `rsx-cast/src/cast.rs:77`
`SEND_RING_FRAME_BYTES=128`, but `FillRecord`/`BboRecord`/`OrderAcceptedRecord`
are 128-byte payloads (rsx-messages asserts) → total 16+128=144 > 128 → the send
path (cast.rs:261/318) takes the "large record" branch that zeroes the ring slot
(never caches). Every NAK for those (the 3 most-sent record types) misses the
in-memory ring and falls to a disk `read_record_at_seq`. The constant's comment
(cast.rs:72-77 "all <= 64 bytes payload … with headroom") is stale/false. Not a
correctness loss (disk fallback still works, modulo the WAL-ROTATE bug) but the
documented fast-path recovery cache is silently inert for the hottest traffic.
Fix: size the ring frame to cover current records (>=144).

### CLI-PTR-READ-UNALIGNED-UB — aligned ptr::read on unaligned WAL buffer (HIGH, soundness)
**Status: OPEN.** `rsx-cli/src/main.rs:343` (and every decode arm: 381,408,439,
468,499,519,537,574,595,619,643,674,700) uses `std::ptr::read(payload.as_ptr()
as *const _)` on `#[repr(C, align(64))]` records read from a `Vec<u8>` payload
(heap alloc, 8–16-byte aligned) — `ptr::read` requires the source satisfy the
type's alignment; violating it is UB. The canonical decoder
`rsx-cast/src/encode_utils.rs:53-55` uses `std::ptr::read_unaligned` for exactly
this reason; the CLI hand-rolled the decode with the wrong primitive. Can
produce garbage fields / crash under non-16-aligning allocators or higher opt
levels, undermining the WAL-inspection tool. Fix: `read_unaligned`.

### GW-CANCEL-NO-RATELIMIT — Cancel bypasses rate limit + circuit breaker (MED)
**Status: OPEN.** `rsx-gateway/src/handler.rs:547-649` the `WsFrame::Cancel` arm
calls none of `ip_limiter`, per-user `RateLimiter`, or `circuit.allow()` — unlike
the `NewOrder` arm (314-364). A client can flood `{"C":[...]}` frames straight to
risk/ME casting with zero throttling, even while throttled/tripped on the order
path. DoS gap on the casting channel. Fix: gate Cancel through the same limiters.

### GW-CANCEL-NOT-USER-SCOPED — cid collision breaks self-cancel (MED)
**Status: OPEN.** `rsx-gateway/src/handler.rs:580,618` `find_by_order_id` /
`find_by_client_order_id` scan the process-global `pending` queue with NO user
filter, and `build_cancel` (584,622) sends the requester's `user_id` + the found
order's id. cid is client-chosen and unnamespaced, so two users can collide:
user A's self-cancel-by-cid can find user B's same-cid order first → ME composite
key `(A, B_oid)` misses → silent no-op → A cannot cancel its own resting order.
Unauthorized cancel of B is blocked by the ME key (incidental), but the missing
user-scope + self-cancel breakage are real. Fix: scope the pending lookup to
`user_id`.

### RISK-DEDUCT-FEE-UNCHECKED — lone non-saturating ledger op (LOW)
**Status: OPEN.** `rsx-risk/src/account.rs:22` `deduct_fee` uses `self.collateral
-= fee` while every other money op in the crate uses saturating/i128-widened
arithmetic. If `collateral` and `fee` sit at opposite i64 extremes it overflows
(debug panic = DoS; release wrap = manufactured solvency). Reaching it needs
absurd values that order-entry notional caps prevent — latent consistency /
defense-in-depth gap, not live-impact. Fix: `saturating_sub`.

### MARK-PARSE-NEG-ZERO — sign lost for "-0.x" price strings (LOW, dead on real feeds)
**Status: OPEN.** `rsx-mark/src/source.rs:275-303` `parse_price("-0.5")`: `whole
= "-0"` parses to `0`, so `whole_val == 0` takes the add branch → `+frac` instead
of `-frac` (sign flip). Dead on real CEX spot feeds (never negative); flagged as
a latent edge only. Fix: track the sign separately from `whole_val`.

### LATENCY-TRACE-STAGES-AGGREGATION — mixed-population median subtraction (MED)
**Status: FIXED 2026-07-04 (`8b33bbc`).** My initial diagnosis was WRONG: I
claimed the return leg (`risk_out`/`gateway_out`) never emits and that taker
completions bypass `route_fill`. An opus trace disproved both — 9/9 spaced taker
FILLS emit `risk_out` (risk main.rs:560) AND `gateway_out` (route.rs:68 via the
real fill path ME→risk RECORD_FILL→forward_to_gw→gateway route_fill), all
anchored on the shared t0. The "zero samples" I saw were: (a) my probes RESTED
(a resting limit / IOC-cancel emits no Fill → correctly no egress sample), and
(b) the **real bug**: `/api/latency-stages` took independent per-stage medians
across a MIXED oid population then subtracted them — cold/rested orders carry
only forward stages (huge cold `me_in`≈5200µs), filled takers carry the return
leg, so subtracting medians from different oid sets yielded a meaningless egress
delta (`risk_out` clamped to 0, `me_out` polluted to ~5ms). Fix (Python only,
server.py `_segment_deltas`/`_cumulative_from_deltas`): each segment's median is
computed over exactly the oids carrying BOTH its endpoints. Sparse capture was
the token-bucket rate limiter (state.rs:69, cap 10 / 1 token per 100ms) dropping
rapid probes before `gateway_in` — space probes ≥1s. **Verified egress**
`me_out→gateway_out` per-oid: `[35,41,43,45,231,1846,2662,3437,6541]µs` —
BIMODAL: fast ~35-45µs (gateway spinning) + ms-scale parked tail (the
POLL_ADD-relevant signal). Caveat: verified by running the exact new computation
against real logs (the live playground couldn't be reloaded without risking the
child trace daemons). **Remaining (deferred, wire change):** non-fill records
(ORDER_INSERTED/DONE/CANCELLED/FAILED) carry only ME-emit `ts_ns`, not an origin
timestamp, so they can't compose into the forward-leg profile without adding an
origin-ts field to rsx-messages + WAL + parser; only the taker-fill leg
(`FillRecord.taker_ts_ns`) composes today. GATEWAY-LATENCY egress is now
measurable for fills; the bimodal tail is the next thing to chase.

## RETURN-PATH-INTERMITTENT-DROP — was a test-fixture bug, not a gateway/risk drop (RESOLVED)

**Status: RESOLVED 2026-07-04.** Root cause was a TEST FIXTURE bug, NOT a
gateway/risk/casting defect. `cluster::seed_book` posted a maker BUY @ 60_000,
but the shared long-lived book (rebuilt from WAL, never reset) already carries
resting asks ~50_000 — so the "maker" CROSSED those asks and filled instead of
resting as a bid. The crossing taker then had no resting bid to hit → no fill →
`wait_for(fills==1)` timed out, which read as a dropped return-leg. Fixed by
seeding the maker BELOW the asks (49_000) with qty matched to the taker so it
rests as the best bid and self-cleans on the fill (`rsx-tui/tests/support/
cluster.rs` + `e2e_orders.rs` price bands all <50_000 + a `LIVE_BOOK` mutex
serialising the two book-sharing fill tests). Verified: `submit_ioc_fills` +
`order_lifecycle_accepted_then_done` PASS against the live cluster. The
mid-investigation "confirmed real, persistent-WS-specific" call was WRONG — the
persistent-vs-transient difference was a coincidence (the playground probe used
a correct maker-sell/taker-buy cross; the e2e used a broken maker-buy).
Residual, NOT this bug: (a) casting is UDP so an occasional order/event
genuinely drops by design (`rsx-matching` FAULTED, "clients re-send dropped
pre-ack orders") — the tests carry a resubmit-once retry; (b) test-infra: the
long-lived shared book means a mid-way-failed run can leave a resting bid that
pollutes the next run's level — matched-qty self-cleaning avoids this in steady
state; a fresh cluster is pollution-free. Original (wrong) triage below.

**[SUPERSEDED — original triage guessed the root cause wrong]** Found writing `.ship/33-TUI-SPEED-TESTS` T3
(`rsx-tui/tests/e2e_orders.rs`) against the live minimal cluster
(gw-0/risk-0/me-pengu, symbol_id=10). Repro: two separate WS connections
(distinct seeded `user_id`s, e.g. 2 and 3), maker posts a resting GTC that
rests fine (`me-pengu.log` `me_in..me_out` completes, gateway relays the
`U` status=1 accept — confirmed working every time), then a second
connection submits a lot-aligned crossing IOC. `me-pengu.log` shows the
crossing order fully processed (`me_in -> me_dedup_done ->
me_wal_accepted_done -> me_match_done -> me_wal_events_done ->
me_index_done -> me_out` all present — ME believes it matched and emitted
Fill+Done), but `risk-0.log` shows only the inbound `risk_in` for that
oid and nothing after (no `risk_out`/`risk_cast_send_done`), and
`gw-0.log` shows only `gateway_in`, no `gateway_cast_recv` — the taker's
own `WsConn` never receives a `Fill` or terminal `U` frame and hangs until
test timeout. Independently reproduced outside the Rust suite with raw
`wscat` sessions against the same live cluster (see repro commands in the
session that filed this entry): a first order on a fresh connection got
no reply within 2s; an identical resubmit (same price, fresh cid, new
connection) was acked within the same short window. Ruled out: not
`InsufficientMargin` (all repros use `_SEED_USERS` with ample collateral),
not tick/lot misalignment (validated multiples of `lot=100000`), not the
gateway's `NewOrder` rate limiter (that path `send_error`s explicitly —
code 1006 "rate limited" — before minting an oid or logging
`gateway_in`; every failing oid here already has a `gateway_in` line, so
it passed rate-limit/circuit checks). Root cause not isolated — candidates
worth checking: WAL dedup replay on a resubmitted cid returning the
original accept silently without re-emitting a cast event; the in-progress
"risk return path RESPEC'd → ME→GW-direct" migration noted in project
memory (partial, not fully implemented) leaving the old ME→Risk→GW leg
half-wired for the fill/done case specifically (accept-path via risk
still works; fill-path does not); or plain casting/UDP loss on the
ME→Risk leg specifically for fills under the concurrent multi-connection
load this test suite generates. `rsx-tui/tests/e2e_orders.rs`'s
`submit_ioc_fills`/`order_lifecycle_accepted_then_done` work around this
with a resubmit-once retry (matching `rsx-matching`'s own documented
mitigation, "clients re-send dropped pre-ack orders (WAL dedup =
exactly-once)"), but even that isn't always enough — both tests can still
fail against this cluster instance. Not a T3 test-file defect; do not
"fix" by weakening the test assertions.

- **Severity:** high
- **Scope:** rsx-risk / rsx-matching / rsx-gateway return-path (ME→Risk→GW)
- **Affected:** fill/done confirmation delivery to the ordering client
- **Source:** `log/me-pengu.log`, `log/risk-0.log`, `log/gw-0.log` around
  2026-07-04T11:08-11:13 (oids `019f2cd110b679719c02d72391586007`,
  `019f2cd4930f7121bea15cf37753ad93`, `019f2cd4c6ad7cd1829930d0494aa843`,
  `019f2cd4da3b7ef3aea4fbf33401b3fa` and others); see also `.ship/
  33-TUI-SPEED-TESTS` session transcript for the raw `wscat` repro
- **Status:** open
- **Fix:** —

## VERIFY-WAL-FILLS-ALWAYS-ZERO — playground /api/verify never sees real WAL fills (LOW)

**Status: OPEN.** `_run_invariant_checks`'s "Fills precede ORDER_DONE (per
order)" check (`server.py:4631-4689`, `_wal_stream_dirs()` scan) reports
`"WAL fills=0 but session fills=183 — sources disagree"` (status `fail`)
even immediately after a real fill was driven through the gateway and
confirmed on the wire (`GwEvent::Fill` observed by an `rsx-tui` `WsConn`
client, T4 `.ship/33-TUI-SPEED-TESTS`). The ME's actual WAL directory is
`RSX_ME_WAL_DIR=./tmp/wal/pengu` (confirmed via `/proc/<me-pid>/environ`
+ `find`, landing at `tmp/wal/pengu/10/10_active.wal`, which does grow on
fills), but the playground's own WAL-dir resolution apparently looks
elsewhere and finds nothing, permanently reporting 0. The "session fills"
counter (Python-local, 183 in the same run) only counts orders submitted
through the playground's own REST endpoints, not real WAL state, so it
isn't a substitute either — net effect, this check is not a usable fill-
durability oracle for orders submitted via any route today.

- **Severity:** low
- **Scope:** rsx-playground/server.py `_run_invariant_checks`
- **Affected:** `/api/verify`, `/api/verify/run-json`, `/verify` page
- **Source:** rsx-playground/server.py:4631-4689; observed via
  `rsx-tui/tests/e2e_guarantees.rs`'s `fill_durability_recorded_in_wal`
  test, which works around it by reading the ME's active WAL file size
  directly instead of this endpoint.
- **Status:** open
- **Fix:** —

## DEMO-TRADE-SUBMIT-ORDER-404 — scripts/demo-trade.sh posts to a route that no longer exists (MED)

**Status: OPEN.** `scripts/demo-trade.sh` submits its maker/taker demo
pair via `curl -sf -X POST "${PLAYGROUND}/api/submit-order" ...`, but no
such route exists in `rsx-playground/server.py` today (`@app.post("/api`
shows `/api/orders/test`, `/api/orders/quick`, `/api/orders/random`,
`/api/orders/batch`, `/api/orders/{cid}/cancel` — no bare
`/api/submit-order`). Live probe: `curl -s -w '%{http_code}'` against
that exact path returns `404 {"detail":"Not Found"}`. The script's
`curl -sf` swallows the 404 silently and falls back to `echo "{}"`, so
its maker/taker submissions are silent no-ops; its actual pass/fail
signal comes only from the later WAL-file-growth poll, which happens to
still pass if something else already crossed the book on the shared
long-lived book — otherwise it hangs to its own 30s timeout and reports
`FAIL: no fill in WAL after 30s` with no hint the real cause was a 404 on
the submit step. Likely the REST route was renamed/removed (to one of
the `/api/orders/*` family above) without updating this script.

- **Severity:** medium
- **Scope:** scripts/demo-trade.sh
- **Affected:** the demo-trade.sh maker/taker submission step
- **Source:** scripts/demo-trade.sh:43-56; confirmed via direct curl
  against a running playground (`start-all minimal`) during T4
  (`.ship/33-TUI-SPEED-TESTS`) test debugging.
- **Status:** open
- **Fix:** —

## MARKETDATA-SHADOW-BOOK-UNBOUNDED-LEVEL-ALLOC [OPEN]
**Severity:** HIGH (crash-loop, takes cluster non-green)
**Where:** rsx-marketdata shadow book construction/recenter — `shadow.rs:29`
`Orderbook::new(config, capacity, mid_price, ...)` sizes the level array from
`mid_price`+`tick_size` via the compression map. `mid_price` for a symbol's
shadow book is derived from replayed events, and is NOT bounds-checked.
**Symptom:** `memory allocation of 47962384944 bytes failed` (repeating) in
`log/marketdata.log`, right after `replay bootstrap complete` (seq 11385).
48 GB = 1,998,432,706 × 24 B (`PriceLevel`) → a ~2-billion-slot level array.
**Root cause (suspected):** a torn/garbage WAL record (from the earlier
ENOSPC + OOM crashes) decodes to an extreme price during replay; the shadow
book is constructed/recentered around that price, so the compression map
computes ~2 B slots instead of ~120k, and the level-array alloc aborts.
**Immediate fix:** clean-state reset (fresh WAL) so replay has no poisoned
record → marketdata bootstraps at mid=50000 (~120k slots). Unblocks the
cluster; does NOT fix the underlying vulnerability.
**Real fix (defer, record only):** bound the shadow book's derived
mid_price / computed slot count — reject or clamp a replay-derived price that
would size the level array beyond a sane cap (e.g. a few million slots), and
harden replay record decode against torn records (length/price sanity).

## PLAYGROUND-MARKET-ORDER-REJECTED-BY-GATEWAY [OPEN]
**Severity:** MED (a whole order type is unusable from the playground)
**Where:** `rsx-playground/server.py` `api_orders_test` market path
(`order_type == "market"` → `price_int = 0`) → gateway.
**Symptom:** submitting a market order (`order_type=market`, `price=0`) via
`/api/orders/test` returns `rejected: price not tick aligned`. The dashboard's
own tick check is correctly skipped for market orders, so the rejection comes
from the **gateway** (an E frame): it treats `price_raw=0` as non-tick-aligned
rather than as a market marker.
**Repro:** `curl -X POST .../api/orders/test -d 'side=buy&order_type=market&price=0&qty=10&user_id=1'`
→ red "rejected: price not tick aligned" (200). A tick-aligned crossing LIMIT
(e.g. buy 51000 vs asks ~50150) fills fine.
**Impact:** market orders don't work end-to-end; the latency test was switched
to a crossing limit as a workaround (`play_latency.spec.ts`).
**Also observed:** an unaligned LIMIT (buy 50201, tick=50) FILLED — the gateway
does NOT strictly enforce tick alignment for limits, yet rejects price-0 market
orders for it. Inconsistent. Needs a spec decision: does the gateway support
market orders (price 0 / a market flag), or is the exchange limit-only?
**Fix (defer, record only):** either (a) gateway accepts a market order marker
(price 0 or an explicit type) and matches at best-opposite, or (b) the
playground sends a crossing tick-aligned limit for "market" and drops the
price-0 path. Decide the product answer first.

## PLAYGROUND-DASHBOARD-STALE-PID-RESTART-RACE [OPEN]
**Severity:** LOW (dev ergonomics)
**Where:** `rsx-playground/playground` restart vs a dashboard started outside
the wrapper (no PID file).
**Symptom:** if a dashboard is launched directly (not via `./playground
start`), `./playground restart` can't see it (no PID file) and aborts on a
stale-PID race; subs had to `kill` by PID and relaunch via the wrapper.
**Fix (defer, record only):** have restart fall back to port-owner lookup
(`ss -ltnp` on 49171) when the PID file is missing/stale.

## GATEWAY-CRASH-UNDER-WS-CHURN (F20) [RESOLVED]
**Severity:** MED (gateway stability under load; cascades into other tests)
**Where:** rsx-gateway, exercised by `play_readiness.spec.ts:189` "gw-0 survives
WS connection churn (F20)".
**Root cause (verified from `log/gw-0.log`):** NOT a connection-handling bug.
The gateway's per-connection path is robust — a direct churn driver ran ~30k
full authed connects + ~20k mid-handshake aborts across distinct users; gw-0
never died, fd peaked at 129 and returned to baseline 9 (no leak on any
disconnect path, including mid-handshake abort and broken-pipe writes). The ONLY
crash signature ever recorded is `main.rs:269` — the cast-receiver UDP rebind
failing with `AddrInUse` (94 occurrences, all one restart storm). Chain: a
supervisor restart overlaps the prior gw-0's `RSX_GW_CAST_ADDR` (:9300) socket
release → fresh gw-0 hits `AddrInUse` on `CastReceiver::new` → retried only
30×200ms=6s then **panicked** → `install_panic_handler` `exit(1)` → supervisor
respawns → re-races the port → tight ~6s crash-storm the test catches as "gw-0
not running during churn" (and pid-churn cascades into the next test).
**Fix:** the overlap trigger was fixed in the supervisor (`f1f2d11`,
start_all idempotency: derive port clear-set from spawn plan incl. 9300/98xx,
poll-for-port-free, detach-before-pkill, reap orphans — verified zero AddrInUse).
Gateway-side hardening (this change): the cast rebind retry budget went 30→100
(6s→20s) with a clearer terminal message, so a transient handover self-heals
instead of hard-crash-storming; a genuinely permanent conflict (two gateways on
one port) still fails fast. See `rsx-gateway/src/main.rs` (`CAST_REBIND_RETRIES`).
**Verified:** churn test PASSES (`play_readiness.spec.ts --grep churn` → 18
passed, gw-0 pid stable through the 90s churn); new binary boots clean (binds
:9300/:8080/:9820 first try); rebind path exercised in isolation reaches
`retry 40/100` with zero panic (old binary died at `retry 30/30` → main.rs:269).
**Also (still open, separate):** `--grep-invert "@long"` did NOT exclude the
@long readiness soak/churn tests in a `bunx playwright test <files>` run — the
tag filter isn't scoping as expected. And make the `play_overview` button test
establish its own clean baseline (Stop All → 0 first) so it can't cascade-fail.

## RECORDER-DEAD-BUT-HEALTHY (durability + false health) [OPEN]
**Severity:** HIGH (a durability demo whose durability is silently broken)
**Where:** rsx-recorder + rsx-health; found by the 2026-07-05 playground audit
(`.ship/40-PLAYGROUND-AUDIT/FINDINGS.md` #1).
**Symptom:** recorder process table / topology / `/component/recorder` / recovery
feed all report **running / healthy / "WAL files found"**, but the recorder log
ends with `BLOCKED: 21 consecutive stream errors exhausted retry budget (20): No
such file or directory` — ME replication (`127.0.0.1:9710`) can't serve old seq
(`56844…58462`), recorder fell behind retention and gave up. ~29 min silent
while every health surface said "fine". Archival replication is dead + invisible.
**Fix (defer, record only):** (1) health must reflect **replication liveness**
(last-consumed-seq advancing), not just pid-alive; a recorder that's BLOCKED must
surface as degraded/red. (2) recorder must **catch up from cold WAL random-access**
when it starts behind the hot retention horizon, instead of exhausting a 20-retry
budget on the live stream. Part of Phase-2 recorder/marketdata → cast quality.

## MKTDATA-DROPS-SHADOW-BOOK-DIVERGENCE [OPEN]
**Severity:** MED-HIGH (real correctness divergence, surfaced by Verify)
**Where:** rsx-marketdata; audit #3/#26.
**Symptom:** Verify FAILs `WAL self-consistency (shadow vs WAL BBO) 1/1 mismatch`;
marketdata logs continuous `WRN seq gap sym=10 expected=N got=N+1` (dropped
casts) and me-pengu `flush took 10-14ms` (>10ms target). The shadow book is
missing events it dropped, so its BBO diverges from the WAL-derived BBO.
**Fix (defer):** mktdata rcvbuf/keep-up (drain the ME casting firehose without
RcvbufErrors); investigate the flush latency; surface a drops/gaps metric.

## MARK-PRODUCES-NO-INDEX / RISK-PHANTOM-POSITIONS / ARCHIVE-WAL-BALLOON [OPEN]
**Severity:** MED (grouped audit findings — see FINDINGS.md #18, #10, #28, #29)
- **#18 mark:** connects Binance but never writes/produces an index — Verify
  SKIPs "no index (mark down)", mark WAL 0 bytes, Risk INDEX 0, while mark shows
  "running" everywhere. Make mark produce/persist, or make health say "no index yet".
- **#10 risk phantom positions:** Risk Lookup shows a large long + PnL for user 1
  with **zero backing fills** (WAL FILL = "no WAL events yet") — stale persisted
  position data. One source of truth; clear/reconcile persisted positions on Reset.
- **#28 archive WAL balloon:** 6.9→10.2 GB in ~7 min from maker quote churn with
  no crosses — drives the Dump OOM + slow flushes + fills disk. Need archive
  retention/rotation so a demo doesn't accrue GB/min.
- **#29 high auto-restart counts** on a "healthy" cluster (gw-0 11, others 7) —
  surface as instability rather than buried in the recovery feed.
**Fix (defer, record only):** these are the Phase-2 recorder/marketdata/gateway
→ cast-quality work; fix carefully, not as dashboard patches.

## CAST-RTT-BENCH-HANGS-AFTER-SEND-REMOVAL [ROOT-CAUSED — duplicate of CAST-RTT-BENCH-DEADLOCKS-ON-LOSS]
**Severity:** MED (bench-only; the shipped library is fine).
**Where:** `rsx-cast/benches/cast_rtt_bench.rs:196-203`.
**Root cause (2026-07-06 read-only audit):** the original hypothesis here
(a `send<T>`-removal regression) is WRONG — `send_framed` populates the NAK
ring byte-identically to the removed path (verified against `git show
bb6c1a0`). The real cause: side A's reply-wait is an unbounded
`loop { if Data break; spin_loop() }` with no timeout, no `tick()`, no
`recv_control()`. One dropped A→B loopback datagram under Criterion's
high-rate warmup → B never echoes → A blocks forever, emitting no heartbeat
and serving no NAK retransmit → permanent two-thread spin-deadlock (matches
the observed ~176% CPU at "Warming up"). Same finding as the other session's
CAST-RTT-BENCH-DEADLOCKS-ON-LOSS (commit 01754ef). Fix belongs in the BENCH
(bound the wait; pump `tick`+`recv_control` inside it), not the frozen lib.
Also noted (LOW, frozen — record only): stale flow-control comment at
cast_rtt_bench.rs:145-149; dangling `CastReceiver::poll` doc links at
cast.rs:510,993-994,1096 (method is `try_recv_with`); `read_record_at_seq`
re-export has 0 external callers; manual header+payload framing duplicates
`encode_record` at replication_client.rs:242-248, replication_server.rs:193-197
and :250-254.
**Fix (defer, record only):** re-run after the in-flight cast.rs refactor
settles; if it still hangs, bisect `cast_rtt_bench.rs`'s client/server ping
loop against pre-`bb6c1a0` to find where the reply stops arriving. Do not
