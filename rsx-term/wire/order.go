package wire

import (
	"encoding/json"
	"strconv"
	"time"
)

// RttUnknown marks an unmeasured round-trip time. Never fabricate 0 —
// 0ns is a real (if implausible) measurement and must stay
// distinguishable from "we never measured this".
const RttUnknown int64 = -1

// Cid renders a per-connection monotonic counter as the 20-char
// zero-padded client correlation id webproto-49 expects.
func Cid(counter uint64) string {
	return fmt20d(counter)
}

func fmt20d(n uint64) string {
	s := strconv.FormatUint(n, 10)
	if len(s) >= 20 {
		return s
	}
	pad := make([]byte, 20-len(s))
	for i := range pad {
		pad[i] = '0'
	}
	return string(pad) + s
}

// EncodeNew renders a new-order webproto-49 text frame:
// {"N":[sym,side,px,qty,"cid",tif,ro,po]}.
func EncodeNew(symbolID uint32, cid string, o OrderReq) string {
	ro, po := 0, 0
	if o.ReduceOnly {
		ro = 1
	}
	if o.PostOnly {
		po = 1
	}
	frame := map[string]any{
		"N": []any{symbolID, uint8(o.Side), o.Px, o.Qty, cid, uint8(o.Tif), ro, po},
	}
	b, err := json.Marshal(frame)
	if err != nil {
		// A map of concrete scalar/string values never fails to
		// marshal; this is an invariant, not a runtime condition.
		panic("wire: EncodeNew: json.Marshal of scalar frame failed: " + err.Error())
	}
	return string(b)
}

// EncodeCancel renders a cancel webproto-49 text frame: {"C":["cid"]}.
func EncodeCancel(cid string) string {
	b, err := json.Marshal(map[string]any{"C": []string{cid}})
	if err != nil {
		panic("wire: EncodeCancel: json.Marshal of scalar frame failed: " + err.Error())
	}
	return string(b)
}

// parseFrame parses one webproto-49 text frame's single {"K":[...]}
// shape into its key and raw array elements.
func parseFrame(text string) (string, []json.RawMessage, bool) {
	var obj map[string]json.RawMessage
	if err := json.Unmarshal([]byte(text), &obj); err != nil || len(obj) != 1 {
		return "", nil, false
	}
	for k, raw := range obj {
		var arr []json.RawMessage
		if err := json.Unmarshal(raw, &arr); err != nil {
			return "", nil, false
		}
		return k, arr, true
	}
	return "", nil, false
}

// IsHeartbeat reports whether text is a webproto-49 heartbeat frame
// {"H":[...]}. The conn layer echoes these back verbatim to keep the
// gateway from dropping the link.
func IsHeartbeat(text string) bool {
	key, _, ok := parseFrame(text)
	return ok && key == "H"
}

// OidTo64 takes the low 64 bits (the last 16 hex chars) of a 32-char
// hex UUIDv7 oid — order_id_lo itself, not a hash, so it is exact for
// any single order. Returns 0 on a parse failure or a string shorter
// than 16 hex chars.
func OidTo64(hex string) uint64 {
	start := len(hex) - 16
	if start < 0 {
		start = 0
	}
	tail := hex[start:]
	n, err := strconv.ParseUint(tail, 16, 64)
	if err != nil {
		return 0
	}
	return n
}

// Accepted is the folded event for a webproto-49 "U" frame with
// status == 1 (RESTING/accepted).
type Accepted struct {
	Oid   uint64
	Order OrderReq
	Cid   string
	RttNs int64
}

// Done is the folded event for a "U" frame with status 0 (FILLED) or
// 2 (CANCELLED) — a terminal, non-reject completion.
type Done struct {
	Oid   uint64
	RttNs int64
}

// Rejected is the folded event for a "U" frame with status 3
// (FAILED), or an "E" error frame.
type Rejected struct {
	Reason string
}

// Fill is the folded event for an "F" frame naming an oid this
// connection submitted.
type Fill struct {
	Oid  uint64
	Px   int64
	Qty  int64
	Side Side
}

// pendingOrder is an order submitted on this connection but not yet
// paired to a gateway-assigned oid.
type pendingOrder struct {
	qty  int64
	side Side
	full OrderReq
	cid  string
	at   time.Time
}

// paired is what a "U" accept records for an oid once claimed: the
// side/order/cid recovered from the matching pendingOrder, plus the
// send time used to compute RTT.
type paired struct {
	side  Side
	order OrderReq
	cid   string
	at    time.Time
}

// maxPending caps the pending-order queue so a connection that never
// gets acked (e.g. a dead link) cannot grow this without bound.
const maxPending = 256

// Folder is the stateful fold over one connection's private-stream
// text frames: it recovers each fill's side (F frames carry no side)
// and each accept's cid/order (U frames carry no cid), pairing on
// qty, FIFO tiebreak, exactly mirroring the removed Rust
// rsx-tui/src/ws.rs WsConn fold (commit 500d440).
type Folder struct {
	oidSide          map[uint64]paired
	pending          []pendingOrder
	UnknownSideFills uint64
}

// NewFolder builds an empty Folder ready to fold a fresh connection's
// frame stream.
func NewFolder() *Folder {
	return &Folder{oidSide: make(map[uint64]paired)}
}

// Sent records a submitted order not yet paired to an oid. Call this
// immediately before/after writing the "N" frame; the send time seeds
// the RTT measured on the first "U" that claims it.
func (f *Folder) Sent(o OrderReq, cid string, at time.Time) {
	if len(f.pending) >= maxPending {
		f.pending = f.pending[1:]
	}
	f.pending = append(f.pending, pendingOrder{qty: o.Qty, side: o.Side, full: o, cid: cid, at: at})
}

// claimPending claims the pending order whose qty matches qty (oldest
// of an equal-qty group), else — no qty match — the oldest pending of
// any qty (FIFO fallback). Returns (entry, true) or (zero, false) if
// nothing is pending.
func (f *Folder) claimPending(qty int64) (pendingOrder, bool) {
	if len(f.pending) == 0 {
		return pendingOrder{}, false
	}
	idx := -1
	for i, p := range f.pending {
		if p.qty == qty {
			idx = i
			break
		}
	}
	if idx == -1 {
		idx = 0
	}
	entry := f.pending[idx]
	f.pending = append(f.pending[:idx], f.pending[idx+1:]...)
	return entry, true
}

func rawStr(raw json.RawMessage) string {
	var s string
	if err := json.Unmarshal(raw, &s); err == nil {
		return s
	}
	return string(raw)
}

func rawI64(raw json.RawMessage) int64 {
	var n int64
	_ = json.Unmarshal(raw, &n)
	return n
}

func rawU64(raw json.RawMessage) uint64 {
	var n uint64
	_ = json.Unmarshal(raw, &n)
	return n
}

func at(arr []json.RawMessage, i int) (json.RawMessage, bool) {
	if i < 0 || i >= len(arr) {
		return nil, false
	}
	return arr[i], true
}

// Fold parses one text frame and folds it against the connection's
// accumulated pairing state, returning the resulting event (and true)
// or (nil, false) if the frame yields no event (heartbeat, unknown
// key, malformed JSON, or an order-update status with no mapping).
func (f *Folder) Fold(text string, now time.Time) (any, bool) {
	key, arr, ok := parseFrame(text)
	if !ok {
		return nil, false
	}
	switch key {
	case "U":
		return f.foldOrderUpdate(arr, now)
	case "F":
		return f.foldFill(arr)
	case "E":
		return f.foldError(arr)
	case "H":
		return nil, false
	default:
		return nil, false
	}
}

// foldOrderUpdate handles {"U":[oid,status,filled,remaining,reason]}.
// status: 0 FILLED, 1 RESTING, 2 CANCELLED, 3 FAILED. The first
// non-reject "U" for an oid pairs it to a submitted order via
// claimPending (by qty = filled+remaining, FIFO tiebreak); a reject
// (status 3) carries no qty and must NOT consume a pending.
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

// foldFill handles {"F":[taker_oid,maker_oid,px,qty,ts,fee]}. Side is
// recovered from whichever oid this connection has paired — the
// gateway pushes the same fill to both sides of a trade, so exactly
// one of taker/maker belongs to this user unless both legs are this
// user's own orders (self-trade), in which case taker_oid wins.
func (f *Folder) foldFill(arr []json.RawMessage) (any, bool) {
	takerRaw, ok := at(arr, 0)
	if !ok {
		return nil, false
	}
	makerRaw, ok := at(arr, 1)
	if !ok {
		return nil, false
	}
	pxRaw, ok := at(arr, 2)
	if !ok {
		return nil, false
	}
	qtyRaw, ok := at(arr, 3)
	if !ok {
		return nil, false
	}
	takerHex := rawStr(takerRaw)
	makerHex := rawStr(makerRaw)
	taker64 := OidTo64(takerHex)
	maker64 := OidTo64(makerHex)

	var ownOid uint64
	var side Side
	if p, known := f.oidSide[taker64]; known {
		ownOid, side = taker64, p.side
	} else if p, known := f.oidSide[maker64]; known {
		ownOid, side = maker64, p.side
	} else {
		f.UnknownSideFills++
		ownOid, side = taker64, Buy
	}

	return Fill{Oid: ownOid, Px: rawI64(pxRaw), Qty: rawI64(qtyRaw), Side: side}, true
}

// foldError handles {"E":[code,msg]}.
func (f *Folder) foldError(arr []json.RawMessage) (any, bool) {
	code := ""
	if r, ok := at(arr, 0); ok {
		code = rawStr(r)
	}
	msg := ""
	if r, ok := at(arr, 1); ok {
		msg = rawStr(r)
	}
	return Rejected{Reason: code + ": " + msg}, true
}
