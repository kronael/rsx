# Critique

Deep audit of all v1 specs. Organized by severity, deduplicated
across components.

## Summary

| Severity | Count | Status |
|----------|-------|--------|
| Critical | 9 | Must resolve before implementation |
| High | 12 | Resolve during implementation |
| Medium | 15 | Accepted risk or deferred |

---

## Critical

### C1. IOC/FOK defined in protocol but not in matching engine

ORDERBOOK.md title says "GTC Limit Orders" and matching algorithm
has no TIF logic. GRPC.md and WEBPROTO.md define TIF enum with
GTC/IOC/FOK. DXS records include `u8 tif` field. OrderSlot struct
has no `tif` or `post_only` field.

**Decision needed:** GTC-only for v1 (remove IOC/FOK from proto
and reject at gateway) or implement IOC/FOK in matching algorithm.

Refs: ORDERBOOK.md section 5, GRPC.md TimeInForce, WEBPROTO.md
section "Time in Force", DXS.md FillRecord.

### C2. CaughtUp semantics ambiguous

DXS.md section 5 defines `CaughtUpRecord { seq, ts_ns, stream_id,
live_seq }` but `live_seq` meaning is undefined. If a record is
appended between "reader exhausts files" and "send CaughtUp",
consumer may miss it or incorrectly assume post-CaughtUp records
are live. RISK.md says "On CaughtUp for all streams: go live" but
unclear if per-symbol or global sync point.

**Decision needed:** Define exact replay boundary -- does consumer
see records at `live_seq` or `live_seq + 1`? Is CaughtUp per-symbol
or global?

Refs: DXS.md section 5, RISK.md replication.

### C3. Mark price unavailability during liquidation

LIQUIDATOR.md section 3 places orders at `mark_price +/- slippage`
but MARK.md section 4 says if all sources are stale, no
MarkPriceEvent is published. RISK.md section 4 says "use index
price from BBO" as fallback. Liquidator spec does not specify
fallback when mark price is unavailable.

If mark price is unavailable AND BBO is empty (halted symbol),
liquidation stalls indefinitely while user is underwater.

**Decision needed:** Liquidator fallback price chain: mark -> index
-> halt? Or force-close at last known price?

Refs: LIQUIDATOR.md section 3, MARK.md section 4, RISK.md
section 4.

### C4. Dedup window unsafe across ME restarts

GRPC.md section 7 says dedup map is in-memory with 5min window.
After ME restart, dedup map is empty. A user whose order was
accepted pre-crash can retry with same order_id and get a second
fill. UUIDv7 makes accidental collision unlikely but intentional
replay is possible.

**Decision needed:** Persist dedup set to WAL (replay rebuilds it)
or accept the gap and document it.

Refs: GRPC.md section 7, RPC.md "Duplicate Order ID".

### C5. No service discovery spec

Risk must connect to all matching engines (one per symbol or
symbol group). Gateway must connect to risk. NETWORK.md says "all
gateways talk to risk; risk talks to all matchers" but no spec for
how components discover each other. Static config, Consul, DNS,
or environment variables -- nothing specified.

**Decision needed:** Static TOML config listing endpoints for v1
(simplest), or service discovery protocol.

Refs: NETWORK.md "Communication Topology".

### C6. Clock sync for funding settlement

Funding settles at UTC 00/08/16. RISK.md assumes wall clock
accuracy but no tolerance specified. If clocks skew >1s between
ME, Risk, and Postgres, funding could: run twice (double charges),
skip intervals, or execute at wrong mark prices.

**Decision needed:** NTP requirement with skew tolerance bound.
Funding idempotency key (interval_id) to prevent double-settle.

Refs: RISK.md section 5, GUARANTEES.md sections 1.4 and 8.5.

### C7. Config propagation crash window

METADATA.md says ME polls configs every 10 minutes and emits
CONFIG_APPLIED. If ME crashes after applying config but before
emitting event, Risk/Gateway never see it. They operate with
stale fees/margins. No bootstrap path for a new Risk shard to
acquire current config if it starts after a config was applied.

**Decision needed:** Persist config version in ME WAL. On replay,
Risk re-derives config state.

Refs: METADATA.md "Application Semantics", CONSISTENCY.md
section 1.

### C8. No deployment/ops spec

No spec for: bare metal vs containers, process supervision,
TLS termination, log rotation, disk layout. RECOVERY-RUNBOOK.md
assumes systemctl but no infrastructure-as-code. Cannot deploy
v1 without building entire ops layer ad-hoc.

**Decision needed:** Write DEPLOY.md covering single-machine dev
topology (enough for v1).

Refs: NETWORK.md "Deployment Topologies".

### C9. Upgrade/schema versioning

DXS.md says unknown WAL version -> fail fast. But no spec for:
ME v1.1 emitting records that v1.0 Risk cannot parse. Postgres
schema changes (new columns) require all Risk instances to
upgrade first. No migration runbook. Cannot upgrade components
independently.

**Decision needed:** WAL record version in header (already exists).
Document upgrade order: Postgres schema -> consumers -> producers.

Refs: DXS.md section 1 (version field), DATABASE.md.

---

## High

### H1. Reduce-only enforcement is per-symbol only

ORDERBOOK.md section 6.5 tracks `net_qty` per (user, symbol).
RISK.md section 6 says "reduce-only: pass through to ME (ME
enforces)". But ME only sees single-symbol position. A user
long BTC-PERP can submit reduce-only sell on ETH-PERP -- ME
allows it (no ETH position), violating reduce-only intent.

Refs: ORDERBOOK.md section 6.5, RISK.md section 6.

### H2. Snapshot-migration mutual exclusion not enforced

ORDERBOOK.md section 2.7 says "Never migrate during a snapshot.
If migration is active, snapshot waits." No mechanism described:
no lock, no flag, no timeout, no deadlock-breaking strategy.

Refs: ORDERBOOK.md section 2.7.

### H3. Auth state machine incomplete

WEBPROTO.md defines `A` frame as "fallback" auth but no spec for:
what if both header auth and A frame provided? Can A frame come
after order frames? Nonce validation undefined. No auth timeout
(client connects but never authenticates). Server may accept
orders before auth completes.

Refs: WEBPROTO.md "A: Auth".

### H4. Backpressure scope: SPSC vs gRPC unclear

CONSISTENCY.md section 3 says "ring full = ME must stall". But
Gateway-Risk uses gRPC with HTTP/2 flow control, not SPSC.
RPC.md says rate-limit exceeded -> ORDER_FAILED. Contradiction:
does slow consumer cause ME to stall or gateway to reject?

Refs: CONSISTENCY.md section 3, RPC.md "Backpressure",
NETWORK.md.

### H5. Tip persistence window vs crash recovery

DXS.md section 6 flushes tip to file every 10ms (batched with
I/O). On simultaneous main+replica crash, last persisted tip may
be stale. Replay from stale tip with non-idempotent consumers
could duplicate position updates.

Refs: DXS.md section 6, RISK.md "Recovery: Both Crash".

### H6. Graceful shutdown not specified

All components assume SIGTERM = crash. No spec for: ME draining
in-flight orders, Risk flushing to Postgres, Gateway notifying
users. CONSISTENCY.md section 5 documents crash behavior but not
clean shutdown (which should lose zero data).

Refs: CONSISTENCY.md section 5.

### H7. Startup ordering dependencies undefined

No spec for boot sequence: Postgres -> ME -> Risk -> Gateway?
Can Gateway accept connections before Risk is ready? Does Risk
wait for all MEs to send CaughtUp? Race conditions during
startup produce undefined errors.

Refs: NETWORK.md, DXS.md section 5.

### H8. Malformed WS frame handling undefined

WEBPROTO.md defines frame format but nothing about: missing
fields, unknown message types, wrong value types, empty arrays.
No error frame sent in response to parse failures. Server may
crash or hang.

Refs: WEBPROTO.md "Frame Shape".

### H9. Liquidation fill vs round escalation race

Liquidation orders go through same SPSC ring as normal orders.
A fill can arrive while liquidator is deciding whether to
escalate. Fill updates position, triggers margin recalc, but
order_done hasn't arrived. Liquidation state machine sees
partial state.

Refs: LIQUIDATOR.md section 4, CONSISTENCY.md section 2.

### H10. Post-max-rounds behavior undefined

Config says `max_rounds = 50`. After 50 rounds if user is still
underwater: does liquidation halt? Continue at max slippage
forever? Force-close at market? Spec is silent.

Refs: LIQUIDATOR.md section 9.

### H11. Recovery snapshot offset ambiguous

ORDERBOOK.md section 2.8 says "replay WAL from snapshot offset"
but doesn't define whether offset is inclusive or exclusive.
Replay from seq=1000 vs seq=1001 can duplicate fills or create
gaps.

Refs: ORDERBOOK.md section 2.8, DXS.md section 5.

### H12. WS/gRPC field mapping implicit

WEBPROTO.md defines `BBO:[sym, bp, bq, bc, ap, aq, ac, ts, u]`.
MARKETDATA.md defines `BboUpdate { bid_px, bid_qty, bid_count,
... }`. Field mapping is positional and implicit. No cross-
reference table. `u` = `seq` stated but other mappings assumed.
B snapshot doesn't specify which array is bids vs asks.

Refs: WEBPROTO.md market data section, MARKETDATA.md.

---

## Medium

### M1. Fee rounding direction unspecified

`taker_fee = qty * price * fee_bps / 10_000` with integer
division. Rounding direction (floor/ceil/round) not specified.
Different implementations diverge on sub-tick fees.

### M2. Best bid/ask tracking during migration

During recentering, two level arrays exist. If best_bid_tick
points to old array, new orders in new array may not update
best_bid_tick correctly.

### M3. User state cleanup grace period vague

ORDERBOOK.md section 6.5 says "e.g., 60s" grace period. No
precise duration. Recovery taking >60s could reclaim state
prematurely.

### M4. Config distribution 10min lag

ME polls config every 10 minutes. Orders matched with stale
config in the window. CONFIG_APPLIED forwarded to Gateway but
no spec for what Gateway does (revalidate pending orders?).

### M5. Cancel cid/oid ambiguity

WEBPROTO.md Cancel accepts `cid_or_oid` but no spec for how
server distinguishes them. Format, length, range undefined.
Collision possible.

### M6. NETWORK_ERROR not in FailureReason enum

RPC.md references ORDER_FAILED(NETWORK_ERROR) and
ORDER_FAILED(RATE_LIMIT) but neither appears in GRPC.md
FailureReason enum (values 0-7).

### M7. Capacity planning absent

No bounds on: symbols per ME, users per Risk shard, sharding
strategy (modulo vs consistent hash), memory/CPU budgets.

### M8. SPSC ring sizing rationale missing

GUARANTEES.md specifies ring sizes (4096, 8192) with no formula.
No metric thresholds for when rings are too small. Different
rings have different sizes without documented rationale.

### M9. Advisory lock edge cases

Postgres advisory lock can be held by dead connection. No spec
for stale lock detection, lease-based renewal, or automated
force-failover.

### M10. Liquidation order in smooshed zone

At 500bps slippage, liquidation order lands in zone 4 (smooshed
ticks). Matching requires linked-list scan checking actual
prices. May not fill within 1s delay, triggering unnecessary
round escalation.

### M11. Replica replication protocol not specified

NETWORK.md mentions replicas (ME replica, Risk replica) but no
spec for replication transport (SPSC? network?), lag detection,
or automatic promotion.

### M12. Health check endpoints not specified

RECOVERY-RUNBOOK.md assumes `/health` exists but no spec for
response format, fields, or semantics (is "slow but processing"
healthy?).

### M13. WAL rotation crash recovery

DXS.md mentions WAL file naming but no spec for what happens if
rotation crashes mid-rename. `rsx-wal-inspect` tool assumed but
not specified.

### M14. Heartbeat collision handling

WEBPROTO.md: server sends H every 5s, client must respond in
10s. But no spec for: how client responds, what happens if both
sides send H simultaneously, whether there's a sequence number.

### M15. Liquidation slippage accounting

When liquidation order fills with slippage, is the loss deducted
from collateral or realized_pnl? Accounting path not specified.

---

## Accepted Tradeoffs

These are known limitations, not bugs:

- **Ingress orders can be lost.** Orders at gateway ingress are
  not WAL'd. On risk crash, in-flight orders are lost. Users
  must resubmit. (CONSISTENCY.md section 5)

- **Backpressure correctness depends on strict stalling.** Ring
  full = ME busy-spins. If stalling is incorrect, data loss.
  (WAL.md "Hard Backpressure")

- **UTC scheduling depends on clock sync.** Funding, config
  effective_at_ms, and staleness sweeps all assume reasonable
  NTP. (See C6 for required bounds)

- **Check-to-fill margin window.** Pre-trade check uses mark
  price that may be 1-2 ticks stale. Liquidation handles
  overshoot. (CONSISTENCY.md section 4)

- **10ms position loss on dual crash.** Risk flushes to Postgres
  every 10ms. Both instances crashing before flush = 10ms of
  position updates lost. Fills are never lost (ME WAL).
  (GUARANTEES.md section 3.2)
