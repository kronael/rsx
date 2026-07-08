// Package conn holds the terminal's gateway transports. This file is the
// offline demo mock: no network. It implements feed.Submitter by recording
// what the UI submits, and provides DemoScript — a scripted feed mirroring
// rsx-tui/src/lib.rs demo_events(), but exercising the DERIVED position path
// (an own-order accept -> fill -> done lifecycle that folds into the
// client-tracked position) rather than a synthetic position event.
// See specs/2/55-terminal.md.
package conn

import (
	"errors"
	"sync"

	"rsx-term/book"
	"rsx-term/feed"
	"rsx-term/wire"
)

// MockGateway implements feed.Submitter offline: it records submitted orders
// and cancels so a test can assert exactly what the UI sent. When Down is
// true, Submit and Cancel both error without recording, so the degraded-link
// path is drivable.
type MockGateway struct {
	Submitted []wire.OrderReq
	Cancelled []string
	Down      bool
	mu        sync.Mutex
}

// errLinkDown is returned by Submit/Cancel when the mock is marked Down.
var errLinkDown = errors.New("gateway link down")

// Submit records o unless the link is down.
func (m *MockGateway) Submit(o wire.OrderReq) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.Down {
		return errLinkDown
	}
	m.Submitted = append(m.Submitted, o)
	return nil
}

// Cancel records cid unless the link is down.
func (m *MockGateway) Cancel(cid string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.Down {
		return errLinkDown
	}
	m.Cancelled = append(m.Cancelled, cid)
	return nil
}

// demoSymbolID is the symbol the scripted feed trades (playground PENGU-PERP).
const demoSymbolID uint32 = 10

// demoOid is the gateway-assigned oid the scripted own-order lifecycle uses.
const demoOid uint64 = 7

// demoRttNs is a plausible positive accept RTT for the scripted order.
const demoRttNs int64 = 10_440

// DemoScript is the offline demo feed. Callers send each element as a tea.Msg,
// paced so the demo visibly streams in. It mirrors rsx-tui demo_events() but
// derives the LONG 14 @ 9998 position from a real own-order lifecycle
// (accept -> fill -> done), then replays two full-split latency samples so the
// speed strip and its p50 / best both populate.
func DemoScript() []any {
	return []any{
		feed.GwUp{},
		feed.MdUp{},
		wire.Snapshot{
			SymbolID: demoSymbolID,
			Bids: []wire.Level{
				{Px: 10_000, Qty: 7, Count: 1},
				{Px: 9_999, Qty: 15, Count: 1},
				{Px: 9_998, Qty: 9, Count: 1},
				{Px: 9_997, Qty: 30, Count: 1},
			},
			Asks: []wire.Level{
				{Px: 10_001, Qty: 5, Count: 1},
				{Px: 10_002, Qty: 20, Count: 1},
				{Px: 10_003, Qty: 8, Count: 1},
				{Px: 10_004, Qty: 12, Count: 1},
			},
			Seq: 1,
		},
		wire.MdTrade{SymbolID: demoSymbolID, Px: 10_001, Qty: 5, TakerSide: 0, Seq: 2},
		wire.MdTrade{SymbolID: demoSymbolID, Px: 10_000, Qty: 3, TakerSide: 1, Seq: 3},
		wire.Accepted{
			Oid:   demoOid,
			Order: wire.OrderReq{Side: wire.Buy, Px: 9_998, Qty: 14, Tif: wire.Gtc},
			Cid:   "00000000000000000001",
			RttNs: demoRttNs,
		},
		wire.Fill{Oid: demoOid, Px: 9_998, Qty: 14, Side: wire.Buy},
		wire.Done{Oid: demoOid, RttNs: wire.RttUnknown},
		// Representative measured splits (see reports/): ME match ~340 ns,
		// internal casting RTT ~7.6 µs, local net ~2.5 µs. Two samples so
		// p50 / best differ.
		feed.Latency{Sample: book.Sample{TotalNs: 10_440, NetNs: 2_500, InternalNs: 7_600, EngineNs: 340}},
		feed.Latency{Sample: book.Sample{TotalNs: 9_710, NetNs: 2_300, InternalNs: 7_100, EngineNs: 310}},
	}
}
