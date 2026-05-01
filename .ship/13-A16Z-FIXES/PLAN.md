# 13-A16Z-FIXES — plan

Source: four parallel skeptical reviews simulating an a16z-crypto
technical evaluation (technical merit, honesty audit, code/security,
competitive). Output captured in conversation; this plan is the
actionable distillation.

The reviewers reached three converging conclusions:
1. Engineering is real; the boring choices won.
2. Honesty culture is rare and visible (`.ship/12-SHOWCASE-HONEST/`).
3. The headline performance and security claims are not yet
   evidence-backed in the code or the docs.

This plan addresses every cited finding. Tiers reflect blast radius
and what unblocks next.

## Tier 0 — correctness bugs (block any production claim)

- **T0.1 WAL append errors swallowed on fill path.**
  `rsx-matching/src/main.rs` lines 395, 422, 433, 442, 452, 545,
  714–716 all do `let _ = wal_writer.append(...)`. A failed WAL
  append on a fill silently drops the record, violating Invariant
  #1 ("Fills precede ORDER_DONE"). Replace with explicit error
  handling: log + crash, since the WAL is the source of truth.

## Tier 1 — security (block real-money pilot)

- **T1.1 CMP source-address filter.** `rsx-dxs/src/cmp.rs`
  `CmpReceiver::try_recv` accepts datagrams from *any* sender;
  `sender_addr` is used only for outbound replies. Anyone with L3
  reach to a Risk/ME/Marketdata UDP port can inject
  `RECORD_ORDER_REQUEST` directly. Add a hard check: `from_addr ==
  self.sender_addr` else drop + warn-once. The "trusted internal
  network" claim becomes enforced, not just documented.
- **T1.2 Gateway per-IP limiter cap.** `rsx-gateway/src/state.rs`
  `ip_limiters: HashMap<IpAddr, _>` grows unbounded. Slow-burn DoS
  via IP rotation (or a poorly-fronted deployment). Add a fixed
  cap (e.g. 10 000 entries) with simple LRU eviction.
- **T1.3 JWT hardening.** `rsx-gateway/src/jwt.rs`: validate
  minimum secret length (32 bytes), enforce `nbf`, and add a
  bounded `jti` replay set. HS256 with a short secret is
  brute-forceable from a single observed token.

## Tier 2 — performance / correctness of the headline claim

- **T2.1 `send_ring` no per-send heap alloc.** Replace
  `BTreeMap<u64, Vec<u8>>` (`cmp.rs:43,131`) with a fixed-capacity
  `VecDeque` (or open-addressed ring) of preallocated frames sized
  to `WalHeader::SIZE + MAX_SEND_RECORD`. Drops a `Vec::to_vec()`
  per send; the "zero heap on hot path" claim becomes true for
  the transport.
- **T2.2 Cancel index.** `rsx-matching/src/main.rs:738–750` is an
  O(n) slab scan per cancel; the comment says cap=1024 but actual
  is 65 536 (stale). Add `(user_id, oid) → handle` map maintained
  on insert/done. Defer if larger than this slot. *(Documented;
  not landing in this pass — needs slab API change and benchmark
  to prove the index doesn't regress p99.)*
- **T2.3 Measured E2E latency under load.** Probe shipped in
  `bded133`. Run a sustained 50k msg/s, capture p50/p99, publish
  in README and bench-baseline.json. *(Requires a running cluster
  + load generator; documented for follow-up.)*

## Tier 3 — design gaps (block long-term operability)

- **T3.1 Schema versioning.** `4-cmp.md:445-455` admits no version
  field. Use the 8 reserved bytes in `WalHeader`: define
  `version: u8` at byte 8, reject mismatched versions on receive.
  Bump on additive changes; reserve coordinated-stop semantics for
  breaking changes. *(Spec change + receiver guard; documented for
  follow-up.)*
- **T3.2 Replica → main promotion refactor.** `rsx-risk/src/main.rs:
  1052-1053` uses `std::env::set_var` (unsound on glibc) and a
  recursive `run_main` call (stack growth per promotion). Rewrite
  as a state-machine loop. *(Risky without full coverage;
  documented.)*
- **T3.3 Tile-architecture honesty.** `45-tiles.md` already self-
  audits (matching = "degenerate tile", risk = full, mark partial,
  gateway/marketdata = monoio reactor). Promote the 2.5/4 framing
  to FEATURES.md and root README. Don't claim a uniform tile
  architecture.

## Tier 4 — honesty culture (block credibility)

- **T4.1 Drop "100% complete".** PROGRESS.md columns + BLOG.md
  prose. Replace with measured-vs-budget framing already used in
  README.md §"What's measured vs what's a budget".
- **T4.2 Reconcile test counts.** Reviewers found actual `#[test]`
  count 906 (docs say 871) and Playwright 412 (docs say 421/421).
  Pick a single source of truth (`make test` output) and refer
  every doc to it.
- **T4.3 Strip absolute paths.** `RECOVERY-RUNBOOK.md` (and any
  other root doc) contains `/home/onvos/...`. Replace with
  `${RSX_ROOT:-./}`-style paths.
- **T4.4 monoio claim.** FEATURES.md and CLAUDE.md state "monoio
  not tokio on hot path." True for gateway and marketdata recv.
  False for matching/risk/mark/recorder. State the truth per
  process.
- **T4.5 Crate-count drift.** PROGRESS says 11; FEATURES says 12;
  workspace = 12 (after the rsx-messages split). PROGRESS update.

## Tier 5 — investor narrative (block fundability)

- **T5.1 Wedge memo.** Draft `WEDGE.md` proposing one wedge:
  exchange-in-a-box SDK with a paying design partner *or* niche
  venue (prediction markets / RWA / regional). *Draft for founder
  review — does not land in this pass.*
- **T5.2 BLOG.md reframe.** Currently a technical brag-doc. Rewrite
  as a narrative: who suffers, what we did, why now. Out of scope
  for code-side ship; left as a follow-up.
- **T5.3 BUSINESS.md.** Pricing model, licensing posture, GTM. Out
  of scope for this pass.

## What this pass commits

In order, one commit per item:

1. T0.1 — `[fix] matching: propagate WAL append errors on fill path`
2. T1.1 — `[security] cmp: filter datagrams by source address`
3. T1.2 — `[security] gateway: bound per-IP rate-limiter map`
4. T1.3 — `[security] jwt: enforce min-secret length + nbf + bounded jti`
5. T2.1 — `[perf] cmp: preallocated send_ring, no per-send heap alloc`
6. T4 — `[docs] honesty pass: drop 100%, reconcile counts, monoio truth, paths`
7. T5.1 — `[docs] WEDGE.md draft for founder review`

Each commit must keep `cargo check --workspace --tests --benches`
and `cargo test --workspace` green.

## Out of scope for this pass (with reason)

- T2.2 cancel index — needs slab API change + p99 bench to verify
  it doesn't regress.
- T2.3 measured E2E — needs running cluster and load generator;
  best run by founder with proper tooling.
- T3.1 schema version — spec change first, then code; one full
  cycle worth on its own.
- T3.2 replica promotion refactor — load-bearing on replication
  E2E test that needs to grow first.
- T5.2/T5.3 narrative + business model — editorial work owned by
  founder.

A follow-up `.ship/14-...` should pick these up in order.
