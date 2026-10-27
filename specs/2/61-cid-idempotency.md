---
status: draft
---

# 61 — Client Order Id Idempotency

A client order id (cid) is the client's own name for one order intent; this
spec makes every resend of a cid resolve to the one order its first send
created. That holds on any connection of the user, after a timeout, and
across a gateway restart. The answer to a resend says "this cid is order X"
and cannot be read as "order X failed".

The gateway owns cids (`rsx-gateway/CLAUDE.md` § Rules). Risk and the ME
never see a cid and stay unchanged. Founder rulings this builds on: the dedup
window is 1 hour, and a retry with the same cid inside it maps to the same
order (`.ship/53-COMPONENT-COMPLETION/PLAN.md` § Founder decisions —
2026-10-21). File anchors are at `98327b0a`.

## Problem

### What the gateway does

- `ClientOrderIds` maps `(user_id, cid)` to the minted order id for
  `WINDOW_NS` = 1 hour (`rsx-gateway/src/client_ids.rs:14`). It is an
  `FxHashMap` plus an insertion-order `VecDeque`, expired by the 100 ms
  housekeeping sweep (`client_ids.rs:40-49`, `rsx-gateway/src/main.rs:273-277`).
- A cid is bound only after a successful forward (`rsx-gateway/src/handler.rs:249`). An `E`
  rejection or a failed forward binds nothing.
- A bound cid is looked up after the rate limits and before validation
  (`rsx-gateway/src/handler.rs:338-340`). It is answered with
  `U[<first oid>,3,0,0,3]`: status 3 FAILED, reason 3 DUPLICATE_ORDER_ID
  (`rsx-gateway/src/handler.rs:400-411`). Nothing is forwarded.
- The binding lives a full hour after the order failed, completed or timed
  out, and it is never extended while the order rests.
- The map is in memory only. A restart empties it, and the ME's dedup key is
  the order id (`rsx-matching/src/dedup.rs:10`). A retry after a restart
  gets a fresh order id, so the ME accepts it as a second order.
- The ME writes the cid slot of `OrderAcceptedRecord` as zeros
  (`rsx-matching/src/order.rs:90`), and `OrderRequest` is a full 64 bytes
  with 3 pad bytes (`rsx-messages/src/lib.rs:405-422`). No durable component
  knows any cid.

### What breaks for a trader

| # | Scenario | What the trader gets | Cause |
|---|---|---|---|
| 1 | Sell 10 on cid A. The socket drops before the first `U`. The client reconnects and resends A. | `U[oidA,3,…,3]`. The client marks its live order failed. oidA rests and fills later. The fold does not know oidA, so it counts each fill as a buy (`rsx-core/wire/order.go` `foldFill`, side defaults to Buy). The position is wrong by 2×qty per fill. | The answer reuses the per-order failure frame. `U` carries no cid, and the first `U` went to the dead socket (`rsx-gateway/src/route.rs:57` emits only to live connections). |
| 2 | The same resend, where the client has already paired oidA. | The resend's pending entry is never consumed, because a status-3 `U` claims no pending order (`order.go` `foldOrderUpdate`). The next order's first `U` with no matching quantity claims the oldest pending entry (`rsx-core/wire/order.go:192-209`), which is the stale resend. That order inherits the wrong side and cid. | Same frame. A pending entry is left behind on every duplicate. |
| 3 | A client restarts its counter (`warm-0`, `s0-1`) within the hour. | Every new order is answered as a duplicate of an order from the earlier run. That order may have a different side, price and quantity. | The repeat is matched on cid alone. No order fields are compared. |
| 4 | `stress_client.py` in one run. | Order 10 is answered as a duplicate of order 1, and so are 100 and 1000: `"s0-1".ljust(20,"0") == "s0-10".ljust(20,"0")`. | The right-pad makes cids collide (`rsx-playground/stress_client.py:167-168`). This is a defect for `BUGS.md`, not for this spec. |
| 5 | `bench_probe` is re-run within the hour. | Every warmup gets a duplicate `U`. The probe takes that old oid as its probe and waits for a fill that never comes, then times out. | It uses `warm-{i}` (`rsx-cli/src/bench_probe.rs:326`) and takes the first `U` (`:268-277`). |
| 6 | An order sits unanswered for 10 s (risk is slow). The sweep sends `U[oidA,3,…,10]`, and oidA then rests. | The client is told FAILED. A resend on the same cid is answered as a duplicate, which by luck is correct. A resend on a new cid makes a second order. Cancel by cid or oid answers `1005 order not found` while oidA rests, because the sweep removed the pending entry (`rsx-gateway/src/route.rs:203-215`, `rsx-gateway/src/pending.rs:124`). A late `ORDER_INSERTED` does not bring it back (`rsx-gateway/src/pending.rs:78`). | TIMEOUT is shown as a definite failure, and cancel lookups only search `pending` (`rsx-gateway/src/handler.rs:255-275`). |
| 7 | Risk rejects cid A for margin. The trader deposits and resends A. | Duplicate answer. The order never reaches risk again. Two different failure reasons arrive for one oid. | The binding outlives a definite failure (`BUGS.md` GW-CID-RESERVED-AFTER-FAILURE). |
| 8 | A GTC order on cid A rests for more than 1 h. The client reuses A for a new order. | A second order is bound to A, and `pending` holds two entries with cid A. Cancel by A finds the oldest one (`rsx-gateway/src/pending.rs:115`), so it cancels the old order instead of the new one. | The binding expires while the order still rests. |
| 9 | The gateway crashes or is redeployed with orders in flight. | Every unanswered cid the client resends becomes a second order. The restart is exactly when clients resend. | The map is lost (`BUGS.md` GW-CID-MAP-LOST-ON-RESTART). Shutdown exits without draining `pending` (`rsx-gateway/src/main.rs:228-237`). |
| 10 | The window fills at the instance limit. | About 306 MB (computed, not measured): a 2^22-bucket table × 41 B plus a 2^22-slot queue × 32 B. When the table doubles, the old and new tables are both held for a moment, and the rehash stalls the reactor that serves every order. | Nothing is preallocated and there is no cap. Hot-path allocation breaks the root `CLAUDE.md` zero-heap rule. |

Attacks that did not hold up:

- The check and the bind are not separated by an `await`: `admit_order` →
  `forward` → `insert` runs inside one borrow on the single reactor thread.
  So two connections of one user cannot both mint an order for one cid.
- A cid of more than 20 bytes, or an empty one, is refused at parse
  (`rsx-messages/src/frames.rs:174-178`), so truncation cannot make two cids collide.
- A duplicate never reaches risk or the ME, so it takes no margin and no ME
  time. That is the right owner for cids.

## Target behavior (proposed)

### The binding

A binding is `(user_id, cid) → (order id, fingerprint)`. The fingerprint is a
64-bit hash of `symbol_id, side, price, qty, tif, reduce_only, post_only`. A
binding is made when an order is forwarded and lives until the latest of:

- 1 hour after it was made. A repeat does not extend it, so a retry loop
  cannot keep a binding alive.
- the order leaving `pending` as `Resting`. A cid stays reserved while its
  order rests, so cancel by cid always names one order.

A binding is dropped early on `ORDER_FAILED` for its order. A definite
failure means the order never entered the book, so a resend is a fresh
attempt that risk evaluates again (GW-CID-RESERVED-AFTER-FAILURE, option b,
for `ORDER_FAILED` only). The timeout sweep keeps the binding, because the
order's fate is unknown.

### Answers to a new-order frame

| Condition | Answer | Forwarded |
|---|---|---|
| cid not bound | The order runs through the checks as today | yes |
| cid bound, fingerprint equal | `E[1011,"duplicate cid","<oid32>","<cid>"]` | no |
| cid bound, fingerprint differs | `E[1012,"cid in use","<oid32>","<cid>"]` | no |

Both answers refuse the frame. Neither one says anything about the state of
`<oid32>`. The original order's own `U`/`F` frames keep flowing as they do
now. A repeat is counted on its own counters (`cid_duplicates`,
`cid_mismatches`), apart from the ME's DUPLICATE_ORDER_ID reject. The check
stays where it is: after the rate limits, before validation, the pending cap
and the breaker.

### Timeout sweep

The sweep's `U[oid,3,…,10]` keeps its frame and code. It means "no answer in
10 s, fate unknown", not "the order does not exist". A timed-out order can
still be cancelled by cid or oid while its binding lives, and a late
`ORDER_INSERTED` or fill for it is routed as usual.

### Restart

- **Store epoch.** The greeting `H` carries a second element: a 16-hex-char
  string that names the binding store. It changes exactly when the bindings
  are lost: a new store, or a client landing on another gateway (the load
  balancer routes by user id, `specs/2/20-network.md:142`, and drops a dead
  gateway, `:469`). The periodic `H` keeps one element.
- **Drain on shutdown.** On the shutdown signal the gateway answers every
  new order with `E[5]` and waits, up to `RSX_GW_ORDER_TIMEOUT_MS`, until
  no `Submitted` entry is left in `pending`. Then it exits. A planned
  restart leaves no unanswered cid behind.
- **Crash.** See [Open decisions](#open-decisions) D1. The recommended
  default is a binding store that survives the process (option b).

### Memory bound and cap

The cap is `RSX_GW_CID_CAP` bindings. Its default is
`RSX_GW_RL_INSTANCE × 3600 + RSX_GW_RL_INSTANCE + RSX_GW_MAX_PENDING`: one
hour of the instance limit, plus its burst (the bucket capacity equals the
rate, `rsx-gateway/src/state.rs:88`), plus the orders that rest past the hour. With the
defaults (1000/s, `rsx-gateway/src/config.rs:89`; 10 000 pending, `rsx-gateway/src/config.rs:82`) that is
3 611 000 bindings.

- All of it is allocated at boot, so no binding allocates on the order
  path and no rehash happens.
- A boot whose cap is below one hour of the instance limit plus burst is
  refused, loudly.
- A full store answers new orders with `E[5]`. It never evicts early,
  because an early eviction silently shortens the window and turns a retry
  into a second order.
- Size at the defaults: under option b, about 352 MB, made of a 64 B store
  slot × 3.6 M (230 MB) plus an index of 2^22 buckets × 29 B (122 MB).
  Memory-only (option a), about 340 MB: entry 48 B + control byte, plus a
  32 B queue slot. Both figures are computed, not measured.
- The gauge `cid_bindings` and its cap are on `/metrics`.

## Wire changes (proposed)

| Frame | Change | Old readers |
|---|---|---|
| `E` | New codes 1011 and 1012, with trailing `oid32` and `cid` strings. 1003 and 1010 are skipped because older docs use them (`.ship/53-COMPONENT-COMPLETION/gateway.md:44,173`). | `rsx-messages/src/frames.rs:279-285` needs at least 2 elements and ignores the rest. The Go fold shows `rejected: 1011: duplicate cid`. `stress_client.py:225` pairs an `E` with its oldest awaiting cid. |
| `H` (greeting only) | A second element: the store epoch, as a string. | `rsx-messages/src/frames.rs:288-292` reads element 0. The Go fold ignores `H` (`rsx-core/wire/order.go:254`). |
| `U` | None. Status 3 with reason 3 means the ME's order-id collision only. | — |

No FailureReason value is added, so the table in `18-messages.md` § FailureReason constants
(0–13) is unchanged. The `E` codes go into the one-code-per-condition table
(`rsx-gateway/ARCHITECTURE.md` § Backpressure & Failure Modes).

## Client requirements

- **Form.** A cid is 1–20 bytes of ASCII `[0-9A-Za-z-_]`, never padded. It
  is unique per user for at least the window plus the life of its order,
  across processes and restarts. The shape is a session part of at least
  40 random bits (8 base36 chars), drawn at process start, followed by a
  base36 counter.
- **Resend only the same cid.** A resend of an unanswered order uses its
  cid, and nothing else. A new cid is a new order, every time.
- **TIMEOUT is not a failure.** After `U[…,3,…,10]` the order may still
  exist. Resend on the same cid, or cancel by that cid.
- **Read 1011 as "this cid is `<oid32>`".** Pair the cid with the oid,
  drop the resend's pending entry, and follow `<oid32>`'s own frames. Read
  1012 as a client bug: the cid names a different order.
- **No resend past 55 minutes** after the first send (5 minutes of margin
  for clock skew and backoff). Past that, reconcile instead.
- **Epoch change.** When the greeting's epoch is not the one the cid was
  sent under, never resend that cid. Reconcile it by fills, positions and
  open orders. Open orders over REST (`GET /v1/orders`, `26-rest.md`) is
  not served today (`rsx-gateway/src/rest.rs:77` serves `/v1/symbols` only).
- **In-repo clients.**
  - `stress_client.py:167-168`: drop the pad, add a session part.
  - `bench_probe.rs:326,348`: session part on every cid.
  - `rsx-glass/specs/12-contract-gateway.md` GW-12 (already requires a
    session part) and GW-31: replace "no auto-retry" with the rules above,
    and add 1011/1012 handling to the fold.

## Rejected

- **Order id derived from `(user_id, cid)`.** A resend after the restart
  carries oidA again, so risk freezes margin for it a second time. The
  freeze map is keyed by order id, so the insert overwrites the original's
  entry (`rsx-risk/src/margin.rs:399-406`). The ME then answers
  DUPLICATE_ORDER_ID, and risk releases the freeze by that id (`:341`).
  The original now rests with no margin reserved. Reusing a cid after the
  ME window gives two live orders one id. It also drops the UUIDv7 time
  order.
- **Re-forwarding a repeat with the first order id.** It breaks in the same
  way at `rsx-risk/src/margin.rs:399-406`, and `route_order_failed` removes the
  original's `pending` entry (`rsx-gateway/src/route.rs:186`).
- **cid dedup in the ME or risk.** It is a second owner for a
  gateway-owned concern. It grows `OrderRequest` beyond 64 bytes on the hot
  path. The ME is per symbol, so it cannot see a cid reused on another
  symbol.
- **A status-4 `U` for the duplicate.** The Go fold claims a pending order
  for every status other than 3, before it switches on status
  (`rsx-core/wire/order.go:266-326`), and with quantity 0 it falls back to the oldest
  pending entry. Old readers would mis-pair. An `E` names no order, so no
  old reader can mistake it for an order state.

## Docs this changes when built

The docs state today's answer (`U[<oid>,3,0,0,3]`, a memory-only map). Building
this changes it to `E[1011]`/`E[1012]`, the epoch and the drain in:
`GUARANTEES.md` §1.0 rows 1–2, §1.3, §2.3, §3.1 and §7 (Gateway row);
`18-messages.md` § Idempotency & Deduplication ("Gateway: client order id");
`29-rpc.md` § Timeout and § Duplicate Order ID; `49-webproto.md` § ACK and
retry and the `E` code table; `37-testing-gateway.md` G27;
`rsx-gateway/ARCHITECTURE.md` § Client Order Ids and the failure table;
`rsx-gateway/CLAUDE.md` § Rules.

## Acceptance criteria

The tests go in `rsx-gateway/tests/handler_cid_test.rs` unless the table
names another file.

| # | Given → then |
|---|---|
| A1 | Same cid and same fields twice → one `ORDER_REQUEST` at risk. The second frame gets `E[1011,"duplicate cid",oidA,cid]`, and no `U` with status 3 is sent for oidA. |
| A2 | Same cid, a different price, side, qty or symbol → `E[1012,…,oidA,cid]`. Nothing is forwarded. |
| A3 | Risk sends `ORDER_FAILED` for oidA, then the same cid is sent → a new order with a new order id is forwarded. |
| A4 | The order times out in the sweep, then the same cid is sent → `E[1011]`. Cancel by cid sends a `CANCEL_REQUEST` for oidA. A later `ORDER_INSERTED` for oidA reaches the user. |
| A5 | oidA rests past the window, then the same cid is sent → `E[1011]`. Once oidA's terminal record arrives (already past the window), the cid binds anew. |
| A6 | A gateway process with a store at `RSX_GW_WAL_DIR` is killed with SIGKILL after forwarding cid A. A fresh process on the same dir gets A → `E[1011]` naming oidA, and the greeting carries the same epoch (`tests/gateway_process_test.rs`; option b). |
| A7 | A fresh process on an empty dir → the greeting carries a different epoch. |
| A8 | SIGTERM with a `Submitted` order → new orders get `E[5]`. The process exits after the order's answer, or after the order timeout. |
| A9 | Store at the cap → the next new order gets `E[5]`, and every bound cid still gets `E[1011]`. Boot with a cap below one hour of the instance limit plus burst is refused. |
| A10 | A zero-heap test over 10 000 new orders with distinct cids, after boot: no allocation on the order path (as `rsx-matching/tests/zero_heap_test.rs`). |
| A11 | `stress_client` with 1000 orders in one run → no `E[1011]`, and two runs back to back → no `E[1011]` (`rsx-playground/tests/`). |
| A12 | `rsx-core/wire` fold: `E[1011,…,oidA,cidA]` pairs cidA with oidA and drops the resend's pending entry. A later `F` for oidA carries the paired side (`rsx-core/wire/order_test.go`). |

## Open decisions

- **D1 — Restart.** Options:
  - **(a)** Memory-only, plus the epoch and the drain. Free, but a crash
    with orders in flight still doubles a client that resends without
    checking the epoch. A careful client reconciles without a cid by
    matching symbol, side, price and qty, and REST open orders is not
    served.
  - **(b) Recommended.** A fixed-size `MAP_SHARED` store file under
    `RSX_GW_WAL_DIR`, one 64 B slot per binding, in mint order. It is
    written before `send_raw` and replayed into the index at boot. A
    store write is a copy into page cache with no syscall. It survives
    SIGKILL, panic and OOM kill, but not a kernel crash or the loss of the
    host, and both of those change the epoch. The cost: a crash between
    the store write and the send leaves a binding to an order that never
    reached risk. A resend of it gets `E[1011]` for an oid that never
    shows up, which is safe (no double) but leaves the client to
    reconcile.
  - **(c)** Rebuild the bindings at boot from each ME's WAL over
    replication. This needs the cid in `OrderRequest`, which has no room
    (`rsx-messages/src/lib.rs:405-422`), and the ME to fill `OrderAcceptedRecord.cid`
    (`rsx-matching/src/order.rs:90`). It survives host loss and failover. It costs a
    wider hot-path record and a gateway that depends on every ME's
    replication at boot.
- **D2 — Release on `ORDER_FAILED`.** Recommended. This reverses the
  current default (a) of GW-CID-RESERVED-AFTER-FAILURE.
- **D3 — Memory at the defaults.** About 340–352 MB at 1000/s. Accept it, or
  lower `RSX_GW_RL_INSTANCE` or the cap. Lowering the cap does not shorten
  the window: a full store refuses new orders.
- **D4 — Fingerprint fields.** The recommendation is the seven order
  fields above. Leaving `tif`, `reduce_only` and `post_only` out would let
  a resend that changes them pass as a duplicate.
