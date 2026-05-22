# CTO Review — RSX (2026-05-22)

Adversarial engineering DD. Source-only audit (no browser, no
playground, no curl). Method: read code → cross-check spec →
oracle pass on three load-bearing modules. Output is intentionally
unbalanced; "Strengths" exists to protect things from refine, not
to soften the criticism.

## 1. Verdict

**No, I would not bet a customer SLA on this codebase today.**

The architecture is intellectually serious and the test depth is
above average for a project this young, but the load-bearing
claims — "<50µs design budget", "zero heap on hot path",
"matching engine never drops events", "10 invariants enforced",
"JtiTracker wired through" — do not survive contact with the
source. Specifically:

- Measured `e2e_us` p50 = **11 780 µs** (235× over budget),
  p99 = **233 447 µs** (4 669× over budget) — see
  `bench-baseline.json`. The "<50µs" number lives only in
  spec prose and docs.
- The "zero-heap hot path" claim is contradicted by per-order
  heap allocations in `dedup.rs` and `order_index` (verified
  by codex pass).
- Risk silently drops fills under ring backpressure
  (`rsx-risk/src/main.rs:601`), which directly violates the
  documented "Position = sum of fills" invariant #4.
- The auth service issues JWTs without a `jti` claim
  (`rsx-auth/src/rsx_auth/jwt_util.py:14-23`), so the
  JtiTracker that was just wired through the gateway
  (`72bd481`) is dormant for every token currently in
  circulation. The replay-defence story is structurally
  there but operationally inert.

Engineering health is much better than fundability would
suggest. The lint hygiene, test breadth, and spec discipline
are real. What's missing is a measurement-backed reality
check on the headline claims and a non-trivial backlog of
silent-drop sites that need either backpressure-stall or
escalation.

---

## 2. Top 5 strengths (don't break these in refine)

1. **Spec-to-code cross-referencing is real, not theatrical.**
   `specs/2/6-consistency.md:179-228` enumerates all 10
   invariants and names the exact enforcement site in code
   for each (e.g. invariant #4 →
   `rsx-risk/src/shard.rs::RiskShard::process_fill`). The
   commit history shows this came from a deliberate refine
   pass (`75dd74b [refine] C12: 10 invariants cross-referenced
   spec ↔ code`). Many invariants then re-appear as doc
   comments at the enforcement site
   (`rsx-book/src/matching.rs:31, 265-268`,
   `rsx-risk/src/shard.rs:260, 858`). Preserve this pattern.

2. **Trust-boundary discipline is codified.** `CLAUDE.md`
   "Trust boundaries" section (CMP is intentionally
   unauthenticated; ME doesn't validate user input) is
   actively enforced. Commit `bde3211` explicitly REVERTS
   a "security" patch (source-IP filtering in CMP) on the
   grounds that the spec already owns that concern at a
   different layer. This is mature engineering culture —
   most teams would have accepted the audit finding and
   added the wrong-layer code.

3. **CMP wire format is honestly documented.**
   `specs/2/4-cmp.md` §8 has a comparison table against
   Aeron, kcp, QUIC, gRPC and is candid about what's
   borrowed and what's simpler ("strictly worse than QUIC
   over the public internet"). §10 enumerates known limits
   (endianness, retransmit horizon = WAL retention, no
   tcpdump). This is the kind of spec you can hand to a new
   hire and they'll know where the rough edges are.

4. **Test depth exists where it matters.** 923 `#[test]`
   functions in workspace (counted from `rsx-*/tests/` and
   `rsx-*/src/`); 43 `#[ignore]` (integration / testcontainers).
   `rsx-matching/tests/invariant_test.rs:44-99` exercises
   "Fills precede ORDER_DONE" by actually constructing a 10-fill
   sweep and asserting event ordering — not a layout check.
   `rsx-dxs/tests/cmp_test.rs::nak_retransmit_from_wal` proves
   the cold-tier WAL fallback works end-to-end.
   `rsx-risk/tests/position_test.rs` tests fill flips, weighted
   entry, exact close — real behavioral coverage.

5. **The 12-crate split is genuinely orthogonal.** `rsx-dxs`
   declares no `rsx-types` dep (`rsx-dxs/Cargo.toml`); domain
   wire records live in `rsx-messages` on top of an
   `rsx-dxs::CmpRecord` trait. This is the unusual case
   where the "domain-agnostic transport" claim survives
   inspection — `WalHeader`, `CmpSender`, `CmpReceiver`
   reference nothing in `rsx-types` or `rsx-messages`. That
   makes the WAL/CMP code reusable outside RSX (which is the
   B+A wedge bet in `.ship/13-A16Z-FIXES/WEDGE.md`).

---

## 3. Top 5 risks (forced rank, severity in brackets)

### R1 — Silent fill drop on risk ingest [critical]

`rsx-risk/src/main.rs:601` — `let _ = fill_prod.push(FillEvent { ... });`

A FillRecord arrives from ME via CMP, the receiver has already
acked it (sequence advanced), and the risk shard's `fill_prod`
SPSC ring is full. The fill is **dropped silently**. The comment
on line 600 calls this "intentional backpressure"; it isn't —
intentional backpressure stalls the producer or escalates an
alert. This is data loss.

Downstream effects:
- Invariant #4 ("Position = sum of fills") becomes false.
- The "tip" for that symbol does not advance for the dropped
  fill, but the CMP receiver's `expected_seq` already advanced
  past it — no NAK will retransmit.
- Position reconstruction from WAL replay on restart would heal
  the gap, but the in-memory shard is silently inconsistent
  until that happens.

The same pattern is used at `main.rs:571` (BBO drop), `:758`
(mark drop), and `rsx-risk/src/shard.rs:1059, 1088, 1093`
(accepted/response). The comment "intentional backpressure"
is repeated verbatim at each site — pattern-pasted from a
single bad template.

**Acceptance test**: under `make integration` or a new test,
fill a fill_prod ring with N+1 fills, assert either (a) the
CMP receiver stalled, OR (b) the dropped fill is replayed
from WAL, OR (c) the shard's metric exports a non-zero
"fills dropped" counter that fails a /health check.

### R2 — Headline latency budget is off by 234× p50, 4669× p99 [critical]

`bench-baseline.json`:
```
e2e_us: { n: 619, p50: 11780.0, p99: 233447.0, ts: 1779449990 }
```

Design budget per `specs/2/4-cmp.md` §9, `ARCHITECTURE.md:200`,
and elsewhere: **<50 µs**. Measured p50: 11.78 ms. Measured
p99: 233 ms.

To the credit of the spec, `specs/2/4-cmp.md:425-435` does say
"treat the 50 µs number as a design budget, not a measurement"
— but `ARCHITECTURE.md`, `BLOG.md` (commit `82f096d` reframe),
and `README.md` all still describe the system in terms of the
50 µs target without quoting the measurement. The honest
disclosure exists in exactly one place; everywhere else the
project speaks as if the budget is the result.

Smaller microbenches in the same file are obvious artifacts:
`cancel_order: 1.3e-11 ns`, `modify_order_qty_down: 3.8e-7 ns`,
`slab_alloc_bump: 7e-4 ns` — these are sub-attosecond figures
where the optimizer deleted the bench body. They need a
`black_box(...)` wrapper or the criterion macro that prevents
constant folding. Until then the per-op numbers (which DO get
quoted in `specs/2/4-cmp.md` §9) are partially noise.

**Acceptance test**: add a release gate that the CI mean
`e2e_us.p50` must be either (a) under a written, documented
limit (e.g. 20 ms for now), or (b) regression-checked against
`bench-baseline.json` with a 10% tolerance. Don't ship docs
that say "<50 µs" until at least p99 < 1 ms.

### R3 — JtiTracker is wired but auth service emits no jti [critical]

`rsx-gateway/src/jwt.rs:107-110`:
```rust
let Some(jti) = jti else {
    return true;
};
```

`rsx-auth/src/rsx_auth/jwt_util.py:14-23`:
```python
claims = {
    "sub": ...,
    "user_id": user_id,
    "email": email,
    "aud": "rsx-gateway",
    "iss": "rsx-auth",
    "iat": now,
    "exp": now + ttl_s,
}
return pyjwt.encode(claims, secret, algorithm="HS256")
```

The gateway's `extract_user_and_record_jti`
(`rsx-gateway/src/ws.rs:171-206`) calls `JTI_TRACKER.record(...)`
and rejects on `"jti replay"` — but only if a `jti` claim is
present. The auth service does not emit one. So **every JWT
currently in circulation passes the replay check**. The
defence-in-depth story for commit `72bd481` ("wire JtiTracker
through ws_handshake") is structurally true but operationally
inert.

Two cheap fixes, neither yet present:
- Auth service adds `claims["jti"] = uuid.uuid4().hex` and a
  short `exp` (already done, 1 h default).
- Gateway treats missing `jti` as a *rejection* (or at least
  a warning + counter), not as a free pass.

**Acceptance test**: integration test that issues a token via
`rsx-auth`, decodes it, asserts `jti` is present, then replays
it against gateway and asserts the second use is 401.

### R4 — Matching engine drops events silently when buffer fills [important]

`rsx-book/src/book.rs:88-94`:
```rust
if (self.event_len as usize) >= MAX_EVENTS {
    tracing::warn!(
        MAX_EVENTS,
        "event buffer full, dropping event",
    );
    return;
}
```

`MAX_EVENTS = 10_000`. One order that sweeps 5 000+ makers
emits 5 000 Fills + 5 000 OrderDones + 1 taker OrderDone +
1 BBO = 10 002 events. The 2 trailing events are dropped,
which means:
- Invariant #1 (Fills precede ORDER_DONE) breaks for the last
  maker (its Fill is dropped, OrderDone may not be).
- Invariant #5 (ORDER_DONE is commit boundary) breaks.
- Risk doesn't see those fills → invariant #4 breaks.
- WAL doesn't see them → replay on restart hides the loss
  forever.

`event_len = 0` is set only at the start of `process_new_order`
(`rsx-book/src/matching.rs:44`); cancel re-uses the buffer.
No batching, no spill. A single large IOC sweep eats the
budget.

10 000 events is plausibly fine for current load, but it's a
silent ceiling that violates the spec's own "matching engine
never drops events" claim (`specs/2/6-consistency.md` key
invariant 4: "Matching engine never drops events (ring full
= stall)"). The code does the opposite — it drops them. The
spec is the load-bearing artifact and it's now wrong.

**Acceptance test**: test that places 5 001 makers at the same
price level, sweeps with one IOC, asserts `events.len() ==
10 003` (5001 fills + 5001 dones + 1 taker done + 1 BBO), and
fails if any event is missing.

### R5 — CMP reorder-buffer overflow and NAK clamp are gap-tolerant by design [important]

`rsx-dxs/src/cmp.rs:706-720` (verified by oracle pass):
```rust
} else {
    warn!("reorder buf full ({}), skip gap {}..{}", ...);
    self.reorder_buf.clear();
    self.expected_seq = seq + 1;
    return Some((hdr, payload.to_vec()));
}
```

When the receiver reorder buffer (default 512) fills, it
**clears all buffered packets** (some of which had lower seq
than what we're about to deliver) and jumps `expected_seq`
past the gap. Any packet that arrived later but had a seq in
that gap is now permanently lost — no NAK will recover it
because `expected_seq` has moved past.

Combined with the NAK clamp at `cmp.rs:281`
(`let count = nak.count.min(SEND_RING_CAPACITY as u64);`),
a NAK for a span of, say, 10 000 records is silently
truncated to 4096 with no fallback. The cold-tier WAL path
(`cmp.rs:313`) is unreachable for the truncated tail. Codex
verified: receiver does not chunk large gaps into multiple
NAKs, so the truncated tail is permanently lost. The comment
"Beyond ring capacity we'd be reading WAL anyway" is wrong
— the code never reads WAL for the clamped tail.

Both of these are *deliberate* per the CMP spec ("loss is
rare on internal LAN"), but they mean CMP/UDP is best-effort
for the producer's GW→ME→GW path. Anything that needs
exactly-once delivery (positions, balances) must reconcile
from WAL on a slow path. As long as the receivers do that
reconciliation, this is a design tradeoff; as soon as a
component assumes CMP delivered everything, it's a bug.

**Acceptance test**: deliberately drop 513 consecutive packets
between ME and Risk, then verify positions reconstructed from
WAL match positions in-memory at risk after a settle period.

---

## 4. Forced rank: 3 fixes I'd do this week

### Fix 1 — Stop silent drops on risk-side CMP ingest

**What**: change `rsx-risk/src/main.rs:601, 571, 758` and the
analogous sites in `shard.rs:1059, 1088, 1093` from
`let _ = ring.push(...)` to:
- on `Err`, set `shard.backpressured = true` AND log at WARN
  with a structured counter,
- block / spin on the next loop iteration until the ring
  drains.

Same template `push_persist` already uses (`shard.rs:185-192`).

**Done when**: ring full on `fill_prod` causes the CMP
receiver to STALL its `try_recv` loop until drained, AND a
soak test confirms positions == sum of fills after sustained
fill flood. Grep `let _ = .*push(` across `rsx-risk/src` and
`rsx-marketdata/src` should return 0 hits outside test code.

### Fix 2 — Emit `jti` from auth and treat missing `jti` as rejection

**What**: two-line patch in `rsx-auth/src/rsx_auth/jwt_util.py`
(`claims["jti"] = uuid.uuid4().hex`), and tighten
`rsx-gateway/src/jwt.rs:107-110` from "pass on missing jti"
to either:
- reject (cleanest), or
- accept with a `jti_missing_tokens` counter wired into
  `/x/health`.

**Done when**: integration test issues a real JWT via the auth
service, replays it twice through the gateway, second attempt
gets 401 with `jti replay`. Add a test that JWT without `jti`
is rejected.

### Fix 3 — Land an honest release gate on `e2e_us`

**What**: write the bench gate that
`specs/2/22-perf-verification.md` already describes. Run
`make perf` in CI, parse `bench-baseline.json`, fail if
`e2e_us.p50` regresses >10% vs baseline OR exceeds a
documented hard ceiling (start at 20 ms; tighten quarterly).

**Done when**: a PR that adds 100 ms to the e2e path fails CI
without anyone needing to read the bench dump. Same gate
strips the `<50 µs` language from `ARCHITECTURE.md`,
`README.md`, `BLOG.md` until the measurement matches the claim
(replace with "design budget; current p50: 12 ms").

---

## 5. Surprises

### Positive

- **Refine-pass culture is visible in git.** Commits with
  prefixes `[refine] A1..C12` show a structured pass at
  hygiene (`caa9a5b sweep let _ on Result returns`,
  `b393b95 replace bare unwrap with expect`,
  `75dd74b 10 invariants cross-referenced`). Even where the
  pass was incomplete (18 `let _ =` remain), the *attempt* is
  documented and committed.
- **Honest spec disclosures.** `specs/2/4-cmp.md:425-435`
  voluntarily flags that "<50 µs is design budget not
  measurement"; §10.7 voluntarily flags the NAK count clamp;
  §10.6 voluntarily flags missing cargo-fuzz target. The team
  writes down what they haven't done. That's rarer than it
  should be.
- **Adversarial audit landed real bugs.** The 28-finding
  playground audit (`.ship/15-PLAYGROUND-AUDIT/FINDINGS.md`)
  surfaced F1 (CMP `AddrInUse` restart loop → WAL truncation)
  and F22 (latency probe matches any fill, not the probe's
  own cid). Both are real Rust/Python bugs, both shipped
  with fixes (`0120806`, `596d24a`), both got regression
  tests. This is the immune system working.
- **rsx-dxs is genuinely reusable.** A WAL+CMP library with
  no domain dep, with a `CmpRecord` trait abstraction, is
  the kind of artifact you can extract into a separate
  open-source release.

### Negative

- **"Audit complete: 0 violations across the workspace"
  (MEMORY.md) is incorrect.** 18 `let _ =` patterns on
  Result/bool-returning calls remain (see grep output above).
  Most are documented "ring full = drop newest" sites that
  weren't intentionally categorized as wisdom violations,
  but if the rule is the rule, these are still violations.
- **The spec drift is asymmetric.** `specs/2/6-consistency.md`
  key invariant 4 says "Matching engine never drops events
  (ring full = stall)". The code at `rsx-book/src/book.rs:88-94`
  drops events when `event_len >= MAX_EVENTS` with no stall,
  no producer backpressure, just a `warn!`. The spec wins
  in repo policy but the code wins at runtime.
- **`bde3211 [revert] cmp: drop source-IP filter`** is a
  correct decision per the trust-boundary rule, but operates
  on the assumption that the L3 firewall is correct. A misconfigured
  firewall, a developer running the cluster on `0.0.0.0:9100`
  for testing, or a containerized deploy with sloppy network
  segmentation all become "anyone with reach to the port owns
  the matching engine". The mitigation belongs in deploy
  docs, not code; verify it's actually written down.
- **Mark/Index price reality.** Per F16/F17 in
  `.ship/15-PLAYGROUND-AUDIT/FINDINGS.md`, the playground
  fabricated `index_px = mark * 1.0001`. That's a playground
  bug (fixed in `47b6fce`), but the underlying observation
  stands: I cannot find a real index-price source feeding
  `rsx-risk` outside `rsx-mark`'s Binance/Coinbase WS. Funding
  rate calc depends on this; verify the production wiring
  before any settlement is real money.
- **The `cancel_order: 1.3e-11 ns` and similar bench
  artifacts** mean part of `bench-baseline.json` is garbage.
  At Pearson-correlated levels it's fine; for any per-op
  budget claim it's noise.

### Expected but absent

- A `fuzz/` directory with `cargo-fuzz` targets on
  `WalHeader::from_bytes`, `protocol.rs::parse`, and
  `wire.rs::OrderMessage` deserialization. The CMP spec §10.6
  explicitly tracks this; the directory doesn't exist.
  (`find rsx-* -type d -name fuzz` → empty.)
- A `LOAD-TEST.md` or equivalent document quantifying the
  ceiling at which fills start dropping. Without it the
  "intentional backpressure" comments are unfalsifiable.
- A property-test crate (proptest / quickcheck) on the
  matching loop. `Cargo.lock` has no `proptest` or
  `quickcheck` entry. For an exchange where the matching
  rules are the product, this is conspicuous.
- A central `metrics.rs` per crate that exports the
  ring-full / drop / panic counters mentioned in
  `MONITORING.md`. The grep `tracing::warn` count is high
  (~200 hits across `rsx-*/src/`) but none are structured
  counters; everything is text-log. `MONITORING.md` claims
  "structured-log metrics shipping"; the structure is mostly
  free-form messages.

---

## 6. Out-of-scope notes (UI gaps observed without opening browser)

These are CEO-lens findings the CTO couldn't act on directly,
flagged so the synthesis pass has them.

- The 28-finding playground audit reveals that
  the dashboard frequently shows fabricated data
  (`/x/core-affinity` invents core numbers from row index,
  F21; `/x/ring-pressure` derives ring fill from WAL lag,
  F25; `index_px` was synthesized from mark, F16). All 28
  are claimed closed; the broader pattern question — "how
  did the dashboard get so far divorced from the cluster?"
  — is a CTO question. The answer appears to be: the
  dashboard was prototyped fast in Python with stubs that
  never got replaced with real wiring. The lesson for code
  review is: any partial endpoint that returns plausible
  fake data is worse than one that returns 501. Consider
  a project rule: stubs MUST 501, never paint green.
- Per `.ship/15-PLAYGROUND-AUDIT/FINDINGS.md`, the audit
  noted `docs nav advertises Infra and API tabs that don't
  exist`. Stale docs are a doc-audit issue but they erode
  trust signals identically to broken code.
- The trade UI Authentication-failed toast on first paint
  (F12) is a UX bug, but it's a symptom of the same JWT
  story — the UI tries to call an authenticated endpoint
  before the user has logged in. A more disciplined frontend
  state machine would prevent the auth attempt until the
  user clicks "log in". Same root cause as R3: the auth
  flow isn't quite finished, just plumbed.

---

## Appendix A — Invariants: claim vs. enforcement audit (sample)

Three of the 10 invariants in `CLAUDE.md` / `ARCHITECTURE.md`
checked against actual code.

### Invariant #1 — "Fills precede ORDER_DONE (per order)"

**Claim**: `specs/2/6-consistency.md:181-184` — enforced by
`rsx-book/src/matching.rs::match_at_level` (emits Fill before
OrderDone) and `rsx-matching/src/wal_integration.rs::write_events_to_wal`
(sequences both into WAL in event-buffer order).

**Verified**: ✅ Yes, mostly. `match_at_level`
(`rsx-book/src/matching.rs:270-364`) does emit Fill before
the maker's OrderDone in the inner loop. `process_new_order`
emits the taker's OrderDone after the matching loop completes.
There's a unit test (`rsx-matching/tests/invariant_test.rs:44-99`).

**Caveat**: violated by `book.emit` silently dropping events
when `event_len >= MAX_EVENTS`. If MAX_EVENTS is crossed mid-sweep,
the dropped events break the "Fills precede ORDER_DONE" sequence.
So the invariant is enforced *up to* the 10 000-event ceiling
and silently violated above it. See R4.

### Invariant #4 — "Position = sum of fills (risk engine)"

**Claim**: `specs/2/6-consistency.md:195-197` — enforced by
`rsx-risk/src/shard.rs::RiskShard::process_fill` calling
`Position::apply_fill` for both taker and maker on every
persisted fill.

**Verified**: ⚠️ Partial. `process_fill`
(`rsx-risk/src/shard.rs:265-445`) does apply fills correctly,
and the seq-dedup gate (`if fill.seq <= self.tips[sid]`)
prevents double-counting. But the invariant requires that
EVERY fill from ME reaches `process_fill`. The CMP-receive
path in `rsx-risk/src/main.rs:601` silently drops the fill
on ring full:

```rust
let _ = fill_prod.push(FillEvent { ... });
```

So invariant #4 holds *if* the `fill_prod` ring never fills.
There is no automated check that ring full doesn't happen.
See R1.

### Invariant #5 — "Tips monotonic, never decrease"

**Claim**: `specs/2/6-consistency.md:198-201` — enforced by
`rsx-dxs/src/client.rs::DxsClient::run_*` (`self.tip = self.tip.max(seq)`)
and `rsx-risk/src/shard.rs::process_fill` (writes seq after
`seq > tip` dedup gate).

**Verified**: ✅ Yes. `rsx-dxs/src/client.rs:305` does use
`.max(seq)`, comment explicitly cites invariant #5. The risk
shard's dedup gate at `shard.rs:273-275` correctly skips fills
with `seq <= tip`. This one is genuinely solid.

---

## Appendix B — Heap-allocation audit on the hot path (sample)

Codex pass on `rsx-matching/src/main.rs` (lines 426-661 main
loop, 664-805 `send_event_cmp`, 847-965 `process_cancel`),
asked: "where does this allocate per order?"

Verified findings:
- ✅ `OrderMessage::to_incoming()` is stack-only
  (`rsx-matching/src/wire.rs:24-` — no `String`/`Vec`).
- ✅ Each `FillRecord`/`OrderDoneRecord`/etc. constructed in
  `send_event_cmp` is a stack local.
- ✅ `cmp_sender.send_raw(...)` reuses the preallocated
  `buf: [u8; PACKET_BUF_SIZE]` (no per-send alloc).
- ❌ `dedup.check_and_insert(...)` at `main.rs:457` →
  `seen.insert(...)` + `pruning_queue.push_back(...)` in
  `rsx-matching/src/dedup.rs:45-47`. Both `FxHashMap` and
  `VecDeque` are heap-backed amortized containers, and
  `DedupTracker::new()` does NOT pre-reserve capacity. So
  every accepted order can trigger a rehash / reallocation.
- ❌ `order_index.insert(...)` in `update_order_index`
  (`main.rs:74-77`) is on every OrderInserted event. Same
  growth pattern — `FxHashMap::default()` starts at zero
  capacity (`main.rs:400-401`).
- ❌ `CmpReceiver::try_recv` (`rsx-dxs/src/cmp.rs:689`)
  allocates `payload.to_vec()` on every in-order delivery.
  Acknowledged in the doc comment (`cmp.rs:547-554`).
- ❌ `RiskShard::positions_for_user` (`rsx-risk/src/shard.rs:237-256`)
  allocates `Vec::with_capacity(symbols.len())` PER ORDER
  in `process_order`. Comment claims "O(this user's positions)",
  which is true for time, but still O(1) allocations per
  call. With one fill per ms per user that's a steady alloc rate.
- ❌ `RiskShard::check_liquidation_for` (`shard.rs:464-501`)
  allocates `Vec<u32> syms` PER FILL.
- ❌ `RiskShard::maybe_settle_funding` (`shard.rs:865-940`)
  allocates `Vec<u32>` from `exposure.users_for_symbol(sid).to_vec()`
  per symbol per settlement interval.

So the "zero heap on hot path" claim is correct *only* for
the matching engine's `send_event_cmp` write path, *not* for
the order-acceptance path (dedup) or the cancel-index path
(order_index) or the risk engine's order/fill paths. The
project's `MEMORY.md` and `ARCHITECTURE.md:184-189` should
quote this more narrowly.

---

## Appendix C — Wisdom-rule compliance audit

`CLAUDE.md` claims "NEVER `let _ = call_returning_result()`"
and `~/.claude/.../MEMORY.md` says "Audit complete: 0
violations across the workspace."

Grep results (excluding `_pad` and tuple drops):

```
rsx-matching/src/main.rs:702,728,752,778,800  let _ = sender.send(...)?;
rsx-marketdata/src/main.rs:535                 let _ = st.push_to_client(...)
rsx-marketdata/src/state.rs:278,301           let _ = self.push_to_client(...)
rsx-marketdata/src/handler.rs:99,115           let _ = is_new / push_to_client
rsx-risk/src/main.rs:571,601,759,968          let _ = bbo_prod/fill_prod/mark_prod/watch.join()
rsx-risk/src/shard.rs:1059,1088,1093          let _ = rings.accepted/response.push(...)
rsx-gateway/src/order_id.rs:16                 let _ = write!(s, "{:02x}", byte)
```

18 sites total. Of these:
- 5 in `rsx-matching/src/main.rs` drop the `bool` flow-control
  signal of `CmpSender::send` (the `?` correctly propagates `Err`,
  but a `false` return — meaning the receiver said stall — is
  silently ignored). This is a soft bug: the ME pretends the
  send succeeded when the receiver advertised a stalled window.
- ~10 in `rsx-risk` and `rsx-marketdata` are the "ring full =
  drop" pattern. These are the silent-loss sites covered by R1.
- 1 in `rsx-gateway/src/order_id.rs:16` is `write!` to a `String`,
  which only fails on OOM; defensible.
- 1 in `rsx-risk/src/main.rs:968` is `watch.join()` (thread
  join), losing the JoinHandle's error; minor.

The audit pass cited in MEMORY.md needs another sweep.
