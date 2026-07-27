# 47 — Exchange evidence

## Goal

Finish the exchange evidence path without adding another benchmark system: carry
every available server timestamp through normal replies, make the canonical load
cross real liquidity, show scale across independent books, exercise every order
in a million-order book within the matching engine's real cascade bound, and
turn those facts into automatic completeness/regression checks. Keep two public
load profiles only—worst reasonable path and its long extreme-tail run—and trim
only Make wrappers that have no repository consumer. Do not modify `rsx-cast`,
push, publish, or claim bare-metal results.

## Architecture / tradeoffs

- Add `risk_in_ns: u64` directly to `rsx_messages::OrderMessage`. The present
  record is exactly 64 bytes and lacks the repository-standard `align(64)`; the
  added field makes the C payload 72 bytes (or 128 bytes if the struct is aligned
  as a hot-path record). Prefer an explicit `#[repr(C, align(64))]` and a 128-byte
  size assertion if that is required by the existing hot-record invariant. This
  spends one extra cache line and UDP payload bytes, but avoids a keyed side
  table, synchronization, expiry, and a second correctness path. Record the
  measured cost; shrinking is a later optimization only if evidence warrants it.
- Risk stamps `risk_in_ns` at receipt and preserves the gateway timestamp already
  carried in `timestamp_ns`. Matching copies both into each fill and completion
  record. No component estimates a missing timestamp, and `gw_out_ns` continues
  to mean reply-construction time—not socket or network egress.
- Make crossing deterministic by deriving valid aggressive IOC prices and fixed
  lot quantities from the live configured symbol/book, with liquidity seeded
  before the timed barrier. Random side/price generation is unsuitable because
  it can complete thousands of IOC orders while producing almost no matching
  samples. The same runner remains open-loop and the same 20-process fan-out is
  retained.
- Parallel-book evidence uses the existing scenario launcher and
  `RSX_STRESS_SYMBOLS`, one connection pinned to one symbol. It reports total and
  per-symbol completions/latency so aggregate throughput cannot hide one idle or
  failed matching engine. It is a second topology measurement inside
  `perf-load`, not another public Make target.
- `MAX_EVENTS = 65_536` permits only roughly 21k fills in one match cascade
  because fills also emit maker completion/BBO events. A million-order book is
  therefore drained by repeated bounded aggressive orders, proving that all one
  million resting orders are reached without pretending a single order can
  produce one million fills. A separate boundary test proves oversize one-order
  cascades fail loudly rather than truncate events.
- Regression checks extend `perf-load`/`perf-e2e-gate`: correctness and timestamp
  completeness are hard failures; latency compares like-for-like server legs to
  a sealed reference with sample counts and the existing tolerance. Missing
  samples can never pass by yielding `null` percentiles.
- Makefile cleanup is evidence-based. A target may disappear only when repository
  search finds no CI, script, documentation, or nested-Make consumer and its
  recipe is already available through a retained canonical target or direct
  focused command. Internal dependency targets stay internal; useful scripts and
  tests stay. `test`, `e2e`, `integration`, `smoke`, `perf`, `perf-load`, and
  `gate` remain the public development surface.

## Steps

### Step 1 — Carry risk ingress through the hot path

Update `rsx-messages/src/lib.rs` to add the timestamp and explicit size/alignment
contract to `OrderMessage`. Update every constructor in messages, gateway, risk,
matching, tests, and benches. In `rsx-risk/src/main.rs`, stamp the accepted copy
at ingress before placing it on the accepted SPSC ring; preserve that value when
constructing the risk→ME message. In `rsx-matching/src/main.rs`, carry it into
the event publication call. Extend `rsx-book/src/event.rs` only as needed for
the taker event metadata, then update `rsx-matching/src/wal.rs` so `FillRecord`
and the matching `OrderDoneRecord` both contain the original gateway, risk,
matching-ingress, and match-completion timestamps. Keep replay/bench callers'
zero-as-unavailable convention. Add focused record-layout, risk-forwarding,
matching-record, and gateway-frame tests; benchmark the enlarged encode/decode
record alongside the existing message benches. Update
`specs/2/59-latency-observability.md` with exact clock points, zero semantics,
record size, and the single-box/shared-clock caveat.

**Gate:** `make check && make test && make fmt-check && make lint`

### Step 2 — Make the canonical workload cross reliably

Replace random non-crossing order generation in
`rsx-playground/stress_client.py` with a deterministic crossing IOC profile. Use
the existing live symbol/BBO/config endpoints or an explicit seeded-price input
from `scripts/latency-publish.sh`; validate tick/lot alignment and fail before
measurement if either side lacks sufficient liquidity. Alternate sides only
when both sides are seeded, otherwise use the known liquid side. Add maker/depth
preflight outside the timed window and include attempted fills, filled orders,
and fill count in the merged result. Extend
`rsx-playground/tests/test_stress_api.py` to prove generated orders cross, each
terminal response correlates to the correct oid/cid, all accepted matching
orders contribute timestamp legs, and 20 partitions retain unique identities
and synchronized pacing. Keep `stress.py` as the single runner and
`perf-load` as the single entry point.

**Gate:** `make api-unit && make perf-load MODE=single N=200 RATE=100 CONNECTIONS=20 USERS=20 PROCESSES=4`

### Step 3 — Measure independent books in parallel

First repair the caller-owned ME→Risk fan-in that the live trio exposed; do not
change `rsx-cast`. Today every ME sends its independent sequence domain to
`RSX_RISK_ME_RECV_ADDR`, while risk binds one `CastReceiver` associated with an
arbitrary first ME and replays every gap through the first symbol's single
`RSX_ME_REPLICATION_ADDR`. This conflates per-symbol sequences and makes a SOL
or BTC gap request hit PENGU's replay server. Implement the following inside
the existing risk/scenario boundary:

1. In `rsx-risk/src/config.rs`, parse explicit symbol-address maps for
   `RSX_RISK_ME_RECV_ADDRS` and `RSX_ME_REPLICATION_ADDRS` (for example
   `10=127.0.0.1:28311,3=127.0.0.1:28304` and matching `97xx` endpoints).
   Reject duplicate symbols, duplicate receive binds, missing cast/replay peers,
   and malformed entries at boot. Retain singular
   `RSX_RISK_ME_RECV_ADDR`/`RSX_ME_REPLICATION_ADDR` only for a one-symbol
   deployment; fail loudly if singular configuration is combined with multiple
   `RSX_ME_CAST_ADDRS`. This is the migration path—no ambiguous first-symbol
   fallback remains in a multi-book process.
2. In `rsx-risk/src/main.rs`, construct one `(symbol_id, CastReceiver,
   replication_addr)` state per ME, each on a distinct UDP receive port. Poll
   these states round-robin in the pinned loop so one busy book cannot starve
   another. Each receiver owns its own expected/delivered sequence, gap, and
   reconnect state; dispatch decoded records through the existing shard and
   gateway-forwarding path. On `Faulted`/`Reconnect`, call the existing
   risk-local replay helper with that state's symbol ID and replay endpoint,
   then reset only that receiver to that stream's returned tip. Refactor
   `rsx-risk/src/failover.rs::handle_replay` to accept the resolved endpoint
   rather than rereading one process-global env variable.
3. Generalize `run_warm_catchup` in `rsx-risk/src/failover.rs` from one consumer
   to all configured `(symbol_id, replay_addr)` streams. Seed every consumer
   from `shard.tips[symbol_id]`; track CAUGHT_UP/applied/live fences separately;
   attempt the advisory lease only after all streams are caught up. After lease
   acquisition, final-drain every stream to a fresh CAUGHT_UP fence before
   entering Live. A disconnect, unavailable replay, or new data clears only
   that stream's caught-up state and blocks promotion until it is current; an
   empty brand-new stream may count as caught up, but an endpoint serving a
   different stream or an unexplained missing historical range must fail rather
   than be treated as empty. Preserve per-symbol tip files so restart/resume
   never advances one book from another book's sequence.
4. In `rsx-playground/runtime.py`, allocate a deterministic risk receive port
   per symbol, pass the matching address to each ME as
   `RSX_RISK_ME_RECV_ADDR`, and pass both complete symbol-address maps to every
   risk primary and warm replica. Update deployment env examples and
   `specs/2/30-scenarios.md`; do not add processes or public Make targets.
   One-symbol scenarios continue to accept the singular variables during
   migration. Add spawn-plan tests proving PENGU/SOL/BTC have distinct receive
   and replay endpoints.
5. Add risk tests for config validation, interleaved independent stream
   sequences, a gap replaying from only the owning ME, fairness under one busy
   receiver, and warm promotion blocked until all streams catch up. Extend the
   existing replication E2E fixture to run at least two WAL servers with
   overlapping sequence numbers, restart one consumer from its own persisted
   tip, inject a gap in one stream, and prove fills/tips for both symbols remain
   correct through final drain and promotion.

**Fan-in gate:** `make check && make test && make integration && make fmt-check && make lint`

With fan-in green, use an existing multi-symbol Playground scenario and its
per-symbol ME processes; do not introduce a benchmark-only daemon topology. Extend
`rsx-playground/stress.py` aggregation to retain per-symbol offered, completed,
filled, timeout, client-latency, and server-leg sample counts while preserving
the aggregate report. Replace the runner's single global lot size with values
resolved per live symbol: the existing trio is PENGU/SOL/BTC (`10,3,1`) and its
lot steps differ. Extend `scripts/latency-publish.sh` to validate that every
configured symbol received and completed work and to record the scenario and
symbol list. Run the same 20-process worst-reasonable workload first on one book
and then across the existing three-book scenario at the same per-book offered
rate. Store reproducible commands, host caveats, aggregate throughput, and
per-book counts in a dated `reports/YYYYMMDD_parallel-books.md`; do not infer
linear hardware scaling from a shared development host.

**Gate:** `make perf-load MODE=single RSX_STRESS_SYMBOLS=10,3,1 PROCESSES=20 && make fmt-check`

### Step 4 — Gate timestamp completeness and server regressions

Extend `rsx-playground/stress.py::validate_results` and its tests so a crossing
run fails unless every accepted completion has `gw_in`, `risk_in`,
`match_done`, and `gw_out`, every filled order has `me_in`, timestamps are
monotone, and aggregate/per-symbol accounting closes. Extend
`scripts/bench-gate-e2e.sh`, `bench-reference.json`, and
`scripts/latency-publish.sh` to compare p50/p99 for the stable server legs
(`gateway→risk`, `risk→ME`, matching, `match_done→gateway`, and total internal)
against the sealed reference and existing tolerance. Require a stated minimum
sample count per compared leg; retain p99.9/max as extreme-tail observations,
not unstable hard gates. Saving a reference remains explicit and cannot happen
after an invalid run. Update `specs/2/22-perf-verification.md` to describe only
the worst-reasonable run and its ten-minute extreme-tail form.

**Gate:** `make perf-e2e-gate && make fmt-check`

### Step 5 — Exercise every order in a million-order book

Add a release-only ignored evidence test or benchmark in `rsx-book/benches/`
or `rsx-matching/benches/` using the existing harness: seed at least 1,000,000
resting orders across a deliberately deep book, verify the initial count/FIFO
sentinels, then submit repeated aggressive orders capped below the derived
`MAX_EVENTS` fill limit until every resting order is filled. Assert exact fill
quantity/count, maker uniqueness, FIFO sentinels, empty final book, valid best
prices, and no slab leakage. Add a focused boundary test showing a single
aggressor that would exceed `MAX_EVENTS` panics with the named invariant rather
than silently dropping records. Document peak memory, elapsed time, build mode,
host, and the crucial distinction between million-order resident depth and the
~21k-fill per-order cascade ceiling in a dated report and the existing book or
matching performance note.

**Gate:** `make test && make perf PERF_FILTER=deep_million && make fmt-check && make lint`

### Step 6 — Compress the Make surface without deleting capability

Inventory root targets against `.github/`, scripts, documentation, nested
`$(MAKE)` calls, and developer entry points. Remove only unused public wrappers
such as duplicated API subsets or presentation aliases when the same action is
already covered by a retained canonical target/direct command; remove their
`.PHONY` entries and stale docs together. Do not remove internal dependency
nodes used by `gate`/`ci`, the canonical seven targets, baseline save/gate
operations needed by performance workflow, or underlying scripts/tests. Keep
`make help` short by reserving `##` descriptions for intended public targets;
underscore internal recipes. Update `README.md`, `specs/2/44-testing.md`, and
`CHANGELOG.md` to match the actual surface.

**Gate:** `make help && make check && make test && make gate`

### Step 7 — Reproduce evidence and close the audits

From clean state, run the one-book worst-reasonable load, the multi-book load,
the ten-minute extreme-tail load, and the million-order evidence case. Save
dated reports with commit, command, topology, host/shared-clock caveats, sample
counts, and failures as well as successes. Run publication packaging checks
only—never registry publication—and audit matching/book docs and demos against
the measurements. Run CTO, user, and hiring evaluations over the actual source,
docs, demo, and reports; fix only concrete in-scope blockers, rerunning the
relevant make gate after each change. Update `CHANGELOG.md`, the minimal status
section in `README.md`, applicable component READMEs/specs, and
`.diary/YYYYMMDD.md` with signed-off versus experimental components and the
condition that all component sign-offs constitute version 1. Remove this ship
directory at close-out because `.ship/` is ephemeral.

**Gate:** `make check && make test && make e2e && make integration && make fmt-check && make lint && make gate`

## Acceptance

- A normal crossing reply exposes non-zero, monotone gateway ingress, risk
  ingress, matching ingress, match completion, and gateway reply-construction
  timestamps; tests trace one order's unchanged values across all processes.
- The canonical 20-process run produces matching-leg samples for every accepted
  filled order, closes all accounting, and rejects an unseeded/non-crossing run.
- One-book and existing multi-book reports show per-symbol work and aggregate
  throughput; no symbol can silently contribute zero samples.
- `perf-e2e-gate` fails fixtures with a missing timestamp, non-monotone leg,
  insufficient samples, broken accounting, or a server-leg regression beyond
  tolerance; invalid runs cannot replace the reference.
- The deep-book evidence starts with at least one million live orders and
  consumes all of them through repeated bounded cascades. The report states
  that one order cannot safely produce a million fills under `MAX_EVENTS`.
- Root `make help` presents a small intentional surface, while every retained
  CI/script/nested-Make dependency still resolves and the full gates pass.
- Dated reports and component status distinguish measured, signed-off,
  experimental, and deferred claims. No `rsx-cast` edit, push, external publish,
  or bare-metal claim occurs.

## Out of scope

- Shrinking `OrderMessage` back to one cache line before benchmarks demonstrate
  a material regression.
- DPDK/AF_XDP, NIC hardware timestamps, PTP deployment, cross-host clock
  correction, or bare-metal/SLA claims.
- Raising `MAX_EVENTS` to permit a million fills from one order; that would add
  hundreds of megabytes of fixed per-book event storage and needs a separate
  architecture decision.
- New public Make targets, a second load generator, or benchmark-only exchange
  services.
- Any `rsx-cast` modification, git push, crates.io/PyPI/npm publication, or
  external release action.
