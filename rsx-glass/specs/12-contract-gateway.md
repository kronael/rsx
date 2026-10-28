---
status: draft
---

# 12 — Contract: gateway order channel

The gateway order channel is the authenticated WebSocket on which the UI
sends new orders and cancels and receives its own order updates, fills,
errors, heartbeats and liquidation notices, as single-key JSON text frames.
This file owns those frames, the client-side fold that pairs them to orders,
the reason codes, and the latency derivation. The public market-data feed is
[13-contract-marketdata.md](13-contract-marketdata.md); REST and assistant
contracts are [14-contract-support-assistant.md](14-contract-support-assistant.md).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`. The
gateway is being edited concurrently; frame shapes below are read from the
committed `rsx-gateway/src/records.rs` at the pin. `rsx-core/wire` (the fold)
survives the deletion of `rsx-term`.

> Spec `specs/2/49-webproto.md` § Order channel documents these JSON frames
> as the live order channel; its protobuf `WireEvent` section is the roadmap
> target, not what the gateway speaks. Glass speaks the JSON frames below
> [doc specs/2/49-webproto.md "Order channel", "Target: protobuf order
> channel"].

## Connection

| ID | Requirement | Source |
|---|---|---|
| GW-01 | Two independent sockets: the private order socket (this file) and the public market-data socket. A market-data outage never touches the order link and vice versa; each reconnects on its own. | [code rsx-term/conn/live.go:131-180], [doc rsx-term/ARCHITECTURE.md "Edge cases"] |
| GW-02 | Auth: `Authorization: Bearer <JWT>` on the WS upgrade. A fresh token is minted for every dial (a token from before a long outage may have expired). A JWT is reusable until `exp`; there is no single-use `jti`. | [code rsx-term/conn/live.go:151-161], [doc rsx-gateway/CLAUDE.md "A JWT is reusable until exp"], [memory owner-decisions-20261021] |
| GW-03 | The order socket's initial dial failing is fatal at startup (no terminal without an order path); the market-data socket's initial failure enters the reconnect loop and the UI starts degraded. | [code rsx-term/conn/live.go:131-180] |
| GW-04 | Reconnect with bounded exponential backoff: 250 ms, doubling, capped at 5 s; a successful read resets it. Close during a backoff or a dial wins: no redial, a socket dialled after Close is closed and discarded. | [code rsx-term/conn/live.go:30-37, 368-401] |
| GW-05 | Every frame write is bounded by 5 s; the UI never blocks on a stalled peer. | [code rsx-term/conn/live.go:26-28] |
| GW-06 | Link state is visible: `GwUp` / `GwDown` drive the link dot and the status line (`connected` / `disconnected`); while down, submit and cancel fail with a reason (`submit failed: conn: gateway link down`). | [code rsx-term/ui/update.go:31-36], [code rsx-term/conn/live.go:47-49] |
| GW-07 | Heartbeat frames `{"H":[ts]}` from the gateway are echoed verbatim to keep the link. | [code rsx-term/conn/live.go:352-356] |
| GW-08 | Production is explicit: no default ever dials a hosted deployment; the hosted gateway is reached only by an explicit URL. Default without config is the offline mock. | [founder 2026-10-21 "Production is explicit … dialled only by an explicit RSX_GW_URL"], [doc rsx-term/CLAUDE.md "Offline by default (RSX_GW_URL=mock)"] |

Gateway limits a client must live within (defaults at the pin;
[code rsx-gateway/src/config.rs:80-95], [code rsx-gateway/src/state.rs:149]):

| Limit | Default | Client consequence |
|---|---|---|
| idle timeout | 10 s (`RSX_GW_IDLE_TIMEOUT_S`) | a client that does not echo `H` is reaped; GW-07 is mandatory |
| heartbeat interval | 5 s | |
| connections per user | 5 | a sixth connection is closed with no `E` frame; glass opens one order socket per user |
| per-user rate | 10 orders/s (`RSX_GW_RL_USER`) | `E[1006]` |
| order timeout | 10 s | the gateway's pending sweep; a timed-out cid stays bound for the 1 h window [BUGS GW-CID-RESERVED-AFTER-FAILURE] |

GW-09: an auth failure (HTTP 401 on upgrade) is shown as an auth failure with
the server's reason, not hidden inside the reconnect backoff (at the pin a 401
is indistinguishable from a down gateway)
[doc .ship/53-COMPONENT-COMPLETION/gateway.md "Missing"].

JWT claims minted by the terminal (HS256 over the shared dev secret):

```go
// rsx-term/conn/jwt.go:21-46 (exact, comments trimmed)
type jwtClaims struct {
	Sub    string `json:"sub"`
	UserID uint32 `json:"user_id"`
	Aud    string `json:"aud"`
	Iss    string `json:"iss"`
	Exp    int64  `json:"exp"`
	Jti    string `json:"jti"`
}

	claims := jwtClaims{
		Sub:    fmt.Sprintf("rsx-term:%d", userID),
		UserID: userID,
		Aud:    "rsx-gateway",
		Iss:    "rsx-auth",
		Exp:    now.Add(jwtTTL).Unix(),
		Jti:    fmt.Sprintf("rsx-term-%d-%d", userID, now.UnixNano()),
	}
```

`jwtTTL` is 1 h [code rsx-term/conn/jwt.go:16-19]. Self-minting from a shared
secret is a dev convenience; identity for a real trader comes from
`rsx-auth`. How glass obtains a trading token in production is
OD-05 in [16-open-decisions.md](16-open-decisions.md).

## Client → gateway frames

```go
// rsx-core/wire/order.go:32-61 (exact, panics trimmed)
// EncodeNew renders a new-order webproto-49 text frame:
// {"N":[sym,side,px,qty,"cid",tif,ro,po]}.
func EncodeNew(symbolID uint32, cid string, order OrderReq) string {
	ro, po := 0, 0
	if order.ReduceOnly {
		ro = 1
	}
	if order.PostOnly {
		po = 1
	}
	frame := map[string]any{
		"N": []any{symbolID, uint8(order.Side), order.Px, order.Qty, cid, uint8(order.Tif), ro, po},
	}
	b, err := json.Marshal(frame)
	…
}

// EncodeCancel renders a cancel webproto-49 text frame: {"C":["cid"]}.
func EncodeCancel(cid string) string {
	b, err := json.Marshal(map[string]any{"C": []string{cid}})
	…
}
```

| Frame | Fields | Gateway validation | Source |
|---|---|---|---|
| `N` | `sym u32, side 0 buy/1 sell, px i64, qty i64, cid string 1–20, tif 0 GTC/1 IOC/2 FOK, ro 0/1, po 0/1` (ro, po optional) | side ≤ 1, tif ≤ 2, cid 1–20 chars | [code rsx-gateway/src/records.rs parse_new_order] |
| `C` | one key: a 32-char key is an oid, a 1–20-char key a cid | other lengths → error | [code rsx-gateway/src/records.rs parse_cancel] |
| `H` | echo of the gateway heartbeat | | [code rsx-term/conn/live.go:352-356] |

| ID | Requirement | Source |
|---|---|---|
| GW-10 | Prices and quantities on the wire are raw i64 tick/lot units; the UI converts only at display and at input parse. | [code rsx-term/ui/format.go:44-76], [doc CLAUDE.md "Fixed-Point Arithmetic"] |
| GW-11 | The send time is captured before the write and recorded as pending only after the write succeeds, so a failed write leaves no phantom pending order a later accept could mis-pair against. | [code rsx-term/conn/live.go:278-310] |
| GW-12 | A cid is unique per user across process restarts within the gateway's 1-hour dedup window. The terminal's `Cid(counter)` restarts at 1 every process, so a restarted terminal's new orders are answered as duplicates of old ones; glass embeds a per-process session component in every cid. | [BUGS CID-CLIENTS-REUSE-CIDS-ACROSS-RESTARTS], [memory owner-decisions-20261021 "Dedup window: 1 hour"] |
| GW-13 | `order.Symbol == 0` routes to the connection's configured symbol; glass always sets the symbol explicitly. | [code rsx-term/conn/live.go:294-297] |
| GW-14 | Cancel is by cid. An order with no cid yet (accepted but unpaired) cannot be cancelled by key and says so (`selected order has no cid yet`); glass MAY cancel such an order by its 32-char oid, which the gateway accepts. | [code rsx-term/ui/update.go:549-552], [code rsx-gateway/src/records.rs parse_cancel] |

## Gateway → client frames

Read from the gateway's own serializer at the pin
[code rsx-gateway/src/records.rs serialize]:

| Frame | Array | Meaning |
|---|---|---|
| `U` | `["oid32", status, filled, remaining, reason, gw_in_ns, risk_in_ns, me_in_ns, match_done_ns, gw_out_ns]` | order update. status 0 FILLED, 1 RESTING, 2 CANCELLED, 3 FAILED. reason: see the reason-slot ruling below. Stamps: Unix ns, 0 = unavailable |
| `F` | `["taker_oid32", "maker_oid32", px, qty, ts_ns, fee, gw_in_ns, risk_in_ns, me_in_ns, match_done_ns, gw_out_ns]` | a fill on one of this user's orders; carries no side and no symbol |
| `E` | `[code, "message"]` | a refusal or protocol error (codes below) |
| `H` | `[ts_ms]` | heartbeat |
| `Q` | `[symbol_id, status, round, side, qty, price, slip_bps]` | liquidation notice for this user. status 0–4 (spec 13 names 0 placed, 1 filled, 2 cancelled, 3 socialized; 4 is accepted by the parser and not named in the docs read) |

The oid is a 32-hex UUIDv7; the client keys orders by its low 64 bits
(`OidTo64`: the last 16 hex chars, exact for any single order)
[code rsx-core/wire/order.go:88-103].

### The fold (reference implementation)

`U` carries no cid and `F` carries no side, so the client pairs each first
non-reject `U` with a pending submitted order by quantity
(`filled + remaining`), FIFO among equal quantities, falling back to the
oldest pending; a reject (status 3) never consumes a pending order. The pending
queue is capped at 256 [code rsx-core/wire/order.go:158-209].

```go
// rsx-core/wire/order.go:266-326 (exact)
func (f *Folder) foldOrderUpdate(arr []json.RawMessage, now time.Time) (any, bool) {
	oidRaw, ok := at(arr, 0)
	if !ok {
		return nil, false
	}
	oidHex := rawStr(oidRaw)
	statusRaw, ok := at(arr, 1)
	if !ok {
		return nil, false
	}
	status := rawU64(statusRaw)
	oid64 := OidTo64(oidHex)

	claimedRtt := int64(RttUnknown)
	if status != 3 {
		if _, known := f.oidSide[oid64]; !known {
			filled := int64(0)
			if r, ok := at(arr, 2); ok {
				filled = rawI64(r)
			}
			remaining := int64(0)
			if r, ok := at(arr, 3); ok {
				remaining = rawI64(r)
			}
			if entry, claimed := f.claimPending(filled + remaining); claimed {
				f.oidSide[oid64] = paired{side: entry.side, order: entry.full, cid: entry.cid, at: entry.at}
				claimedRtt = now.Sub(entry.at).Nanoseconds()
			}
		}
	}

	switch status {
	case 1:
		p, known := f.oidSide[oid64]
		rtt := int64(RttUnknown)
		if claimedRtt != RttUnknown {
			rtt = claimedRtt
		}
		var order OrderReq
		var cid string
		if known {
			order = p.order
			cid = p.cid
		}
		return Accepted{Oid: oid64, Order: order, Cid: cid, RttNs: rtt}, true
	case 0, 2:
		rtt := int64(RttUnknown)
		if claimedRtt != RttUnknown {
			rtt = claimedRtt
		}
		return Done{Oid: oid64, RttNs: rtt}, true
	case 3:
		reason := uint64(0)
		if r, ok := at(arr, 4); ok {
			reason = rawU64(r)
		}
		return Rejected{Reason: "failure_reason=" + strconv.FormatUint(reason, 10)}, true
	default:
		return nil, false
	}
}
```

```go
// rsx-core/wire/order.go:328-368 (exact, field reads trimmed)
func (f *Folder) foldFill(arr []json.RawMessage) (any, bool) {
	…
	takerHex := rawStr(takerRaw)
	makerHex := rawStr(makerRaw)
	taker64 := OidTo64(takerHex)
	maker64 := OidTo64(makerHex)

	var ownOid uint64
	var side Side
	var symbol uint32
	if p, known := f.oidSide[taker64]; known {
		ownOid, side, symbol = taker64, p.side, p.order.Symbol
	} else if p, known := f.oidSide[maker64]; known {
		ownOid, side, symbol = maker64, p.side, p.order.Symbol
	} else {
		f.UnknownSideFills++
		ownOid, side = taker64, Buy
	}

	return Fill{Oid: ownOid, Px: rawI64(pxRaw), Qty: rawI64(qtyRaw), Side: side, Symbol: symbol}, true
}
```

`E` folds to `Rejected{Reason: "<code>: <message>"}`; `H` and unknown keys
(including `Q`) fold to nothing [code rsx-core/wire/order.go:238-259, 370-381].

### Requirements on the fold (glass must meet; the terminal does not)

| ID | Requirement | Source |
|---|---|---|
| GW-20 | **Self-trade.** A fill whose taker AND maker oids are both this user's paired orders folds both legs (a buy and a sell of the same qty: zero net position change, two fills counted) — never the taker leg alone. Tested with an `F` naming two paired oids. | [BUGS TERM-SELF-TRADE-FILL-FOLD] |
| GW-21 | **Unknown side.** A fill naming no paired oid is NOT folded into the position with a guessed side. At the pin the fold assigns `Buy` and counts `UnknownSideFills`, and the UI then applies it to the position — a fabricated position. Glass shows it as an unattributed fill (counted, side `—`), excluded from the position, and surfaces the counter. The `rsx-core` test `wire/order_test.go::TestFolderFillUnknownSidesFallBackToBuy` locks the fabricated side and changes with this requirement. | [code rsx-core/wire/order.go:358-365], [code rsx-term/ui/update.go:106-114], honesty H-1/H-6 in [02-design-system.md](02-design-system.md) |
| GW-22 | **Reason decoded.** A FAILED `U` shows the FailureReason name (e.g. `rejected: FILL_OR_KILL`), not `failure_reason=13`; the table lives in one place and is tested against `rsx_types::FailureReason`. | [BUGS TERM-FAILED-REASON-NOT-DECODED] |
| GW-23 | **Reason slot by status.** The `U` reason byte is read as a FailureReason only under status 3 and as a CancelReason under status 2; under status 0/1 it is ignored. A status-2 `U` carries the `CANCEL_REASON_*` code of its ORDER_DONE: 0 is a user cancel, 2 an IOC remainder, 4 a post-only cross — glass shows a cancel reason only when it is non-zero. | [BUGS GW-U-REASON-SLOT-BY-STATUS] |
| GW-24 | **One terminal.** A user cancel, a post-only cross and an IOC remainder each produce one status-2 `U` for the oid; removing an order by oid stays idempotent. | [code rsx-term/ui/update.go:823-834] |
| GW-25 | **Open orders keyed by oid.** An Accepted for an oid already open updates it in place; it never appends a duplicate row. (At the pin `Accepted` appends unconditionally.) | [code rsx-term/ui/update.go:89-99] |
| GW-26 | **Partial fills.** A status-1 or status-0 `U` carries `filled` and `remaining`; the working-order row shows remaining qty, and a fill reduces it. (At the pin the row keeps the submitted qty.) | [code rsx-gateway/src/records.rs], [code rsx-term/ui/model.go:198-208] |
| GW-27 | **Liquidation notices.** A `Q` frame for this user is shown (symbol, side, qty, price, round, slip, status name) in the status line and the position area; the terminal ignores `Q`. Risk sends one per placed liquidation round (status 0) and one per socialized loss (status 3); glass must not infer a liquidation from the absence of a `Q`. | [code rsx-core/wire/order.go:247-258], [code rsx-gateway/src/records.rs parse_liquidation], [code rsx-risk/src/shard.rs tick] |
| GW-30 | **Fee.** `F.fee` is always 0 today (the fill record carries no fee); glass shows the fee as `—`, never `0`. | [BUGS GW-FILL-FEE-ALWAYS-ZERO] |
| GW-31 | **Retry across a gateway restart.** The gateway's cid → order map is in memory; a retry after a gateway restart is a new order. Glass does not auto-retry submits; after a reconnect it reconciles by fills and positions. | [BUGS GW-CID-MAP-LOST-ON-RESTART], [code rsx-term/ui/update.go:1054-1058] |
| GW-28 | **State after reconnect or restart.** Orders and fills from before a restart are not in the client fold; glass restores open orders, positions and account from the REST reads ([14-contract-support-assistant.md](14-contract-support-assistant.md)) and labels any client-derived figure `~` until restored. | [doc specs/2/55-terminal.md "Data-source honesty table"], [doc specs/2/49-webproto.md "Post-MVP query messages"] |

## Reason codes

FailureReason (`U` status 3, and the refusal class of an `E`)
[code rsx-types/src/lib.rs:65-85], [doc specs/2/18-messages.md "FailureReason constants"]:

| Value | Name | Value | Name |
|---|---|---|---|
| 0 | INVALID_TICK_SIZE | 7 | REDUCE_ONLY_VIOLATION |
| 1 | INVALID_LOT_SIZE | 8 | NETWORK_ERROR |
| 2 | SYMBOL_NOT_FOUND | 9 | RATE_LIMIT |
| 3 | DUPLICATE_ORDER_ID | 10 | TIMEOUT |
| 4 | INSUFFICIENT_MARGIN | 11 | USER_IN_LIQUIDATION |
| 5 | OVERLOADED | 12 | WRONG_SHARD |
| 6 | INTERNAL_ERROR | 13 | FILL_OR_KILL |
|  |  | 14 | ORDER_TOO_SMALL |

Value 8 is NETWORK_ERROR. POST_ONLY_REJECT is a CancelReason (4), not a
FailureReason. `specs/2/18-messages.md` owns this table and
`specs/2/49-webproto.md` points to it. Glass decodes from the `rsx_types`
table above. `FILL_OR_KILL = 13` is a founder-kept wire value
[memory owner-decisions-20261021]; `ORDER_TOO_SMALL = 14` is a founder-approved
wire addition of the same kind.

CancelReason (`U` status 2) [doc specs/2/18-messages.md "CancelReason constants"]:
0 USER_CANCEL, 1 REDUCE_ONLY, 2 EXPIRY, 3 SYSTEM, 4 POST_ONLY_REJECT, 5 OTHER.

`E` codes sent by the gateway at the pin [code rsx-gateway/src/handler.rs],
[doc rsx-gateway/CLAUDE.md "One error code per condition class"]:

| Code | Message | Condition |
|---|---|---|
| 5 | `overloaded` | pending full, open circuit, or the instance limit |
| 8 | `network error` | a failed forward to risk |
| 1001 | `invalid utf8` | malformed frame bytes |
| 1002 | the parse error text (`invalid json`, `missing field: …`, `invalid value: …`, `unknown type: …`) | a frame that does not parse |
| 1004 | `unsupported` | a frame kind the gateway does not accept from clients |
| 1005 | `invalid order id` / `order not found` | a cancel key that does not resolve |
| 1006 | `rate limited` | rate limit only |
| 1007 | `unknown symbol` | symbol not configured |
| 1008 | `price not tick aligned` | px ≤ 0 or off tick |
| 1009 | `qty not lot aligned` | qty off lot |

GW-29: an `E` shows `rejected: <code> <message>` with the code's class name
when known; one code per condition class, never reinterpreted
[doc rsx-gateway/CLAUDE.md].

## Latency

| ID | Requirement | Source |
|---|---|---|
| LAT-01 | RTT (total) is client-measured: from the successful write of `N` to the first `U` that claims it. A negative (unmeasured) RTT is never recorded. | [code rsx-core/wire/order.go:290-293], [code rsx-term/ui/update.go:767-780] |
| LAT-02 | Every operation shows net, internal and engine time from the records' own stamps; a leg is measured or it is `—` (or `·· pending` while the server does not stamp it). | [founder 2026-10-21 "Show the speed: every operation reports net, internal and engine time from the records' own stamps; a latency is measured or it is `—`"] |
| LAT-03 | Derivation from one frame's stamps (zero = unavailable): `engine = match_done_ns − me_in_ns`; `internal = gw_out_ns − gw_in_ns`; `net = RTT − internal` (client clock vs server clock never subtracted across hosts). Cross-hop stamps assume clock sync (PTP in production, one box in dev). | [doc specs/2/59-latency-observability.md "Design"], [BUGS ME-ENGINE-LATENCY-NOT-REPORTED] |
| LAT-04 | Which frames carry stamps today: `F` and the taker's done `U` carry all five; inserted, cancelled and failed `U` carry zeros. A zero stamp leaves that leg pending, never 0 ns. | [BUGS GW-STAMP-LATENCY-LIVE] |
| LAT-05 | Rolling window of the last 128 RTTs; p50 = `sorted[len/2]`, p99 = `sorted[len*99/100]` clamped, best = min; "—" with no samples. | [code rsx-core/book/latency.go] |
| LAT-06 | RTT health colour: Heading ≤ 50 µs, Degraded > 50 µs, Ask > 100 µs. | [code rsx-term/ui/view.go:749-755, 799-810] |

At the pin the terminal sets net, internal and engine to unknown for every
live sample [code rsx-term/ui/update.go:767-780]; only the offline mock
supplies legs [code rsx-term/conn/mock.go:105-109]. Meeting LAT-02/03 is
new work in glass.

## Order lifecycle as seen by the UI

| Event | UI effect | Source |
|---|---|---|
| `Accepted` | add a working order (oid, cid, side, px, qty, symbol); status `order <oid> accepted`; fold RTT | [code rsx-term/ui/update.go:89-99] |
| `Done` | remove the working order; status `order <oid> done`; fold RTT | [code rsx-term/ui/update.go:100-103] |
| `Rejected` | status `rejected: <reason>` | [code rsx-term/ui/update.go:104-105] |
| `Fill` | fills counter +1; fold into the symbol's position (checked i64); status `fill <oid>: <qty> @ <px>` | [code rsx-term/ui/update.go:106-114] |

Invariant (system-wide): fills precede ORDER_DONE; exactly one completion per
order [doc CLAUDE.md "Correctness Invariants" 1-2]. Glass does not rely on
either for display correctness (GW-24).
