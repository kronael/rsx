package mockx

import "rsx-core/wire"

// DefaultSymbolID is the symbol the scripted scenario trades (PENGU-PERP in the
// playground config — the same id conn.DemoScript uses), so the mock lines up
// with the rest of the exchange's dev universe.
const DefaultSymbolID uint32 = 10

// Scenario is the deterministic scripted market a mock server replays to a
// subscriber: a single symbol's heatmap told in the REAL feed frame types
// (wire.Snapshot / wire.Delta / wire.Bbo / wire.MdTrade / wire.MdHeartbeat).
// It exercises every flow the live path must fold — an opening L2 snapshot with
// a walls-profile depth (thin near levels, a heavy wall a couple ticks off each
// touch), single-level deltas that add / replace / REMOVE levels on both sides,
// BBO updates, aggressor-tagged trades straddling the spread, heartbeats, and a
// visible upward price move (the touch walks 9999/10001 -> 10000/10002).
//
// It is a fixture, not a matching engine: the frames are scripted, not derived
// from an order book. seed nudges only the trade sizes (so tests can vary the
// scene without changing its structure); the frame sequence, prices, seqs, and
// timestamps are identical for every seed, so a golden render stays stable.
func Scenario(symbolID uint32, seed uint64) []any {
	s := &script{symbolID: symbolID, rng: seed*6364136223846793005 + 1442695040888963407, tsNs: 1_000_000_000}

	// The opening book: mid 10000, a heavy wall 3 ticks below / 3 ticks above
	// the touch, thinner liquidity around it. Bids descend, asks ascend (the
	// wire order the decoder trusts).
	s.snapshot(
		[]wire.Level{{Px: 9999, Qty: 50, Count: 1}, {Px: 9997, Qty: 900, Count: 3}, {Px: 9995, Qty: 120, Count: 2}, {Px: 9993, Qty: 40, Count: 1}},
		[]wire.Level{{Px: 10001, Qty: 60, Count: 1}, {Px: 10003, Qty: 850, Count: 3}, {Px: 10005, Qty: 140, Count: 2}, {Px: 10007, Qty: 30, Count: 1}},
	)
	s.trade(10001, s.qty(200), wire.Buy) // lift the offer
	s.trade(9999, s.qty(150), wire.Sell) // hit the bid
	s.bbo(9999, 50, 1, 10001, 60, 1)     //
	s.heartbeat()                        //
	s.delta(1, 10001, 0, 0)              // the offer is pulled -> touch lifts
	s.delta(0, 10000, s.qty(80), 1)      // a new bid appears one tick up (the move)
	s.trade(10003, s.qty(500), wire.Buy) // aggressive buy eats into the ask wall
	s.bbo(10000, 80, 1, 10003, 850, 3)   //
	s.delta(0, 9993, 0, 0)               // a far bid is removed
	s.delta(1, 10002, s.qty(45), 1)      // a fresh near ask replaces the lifted one
	s.trade(10000, s.qty(90), wire.Sell) // a sell hits the new bid
	s.heartbeat()                        //

	return s.frames
}

// script threads monotonic seq / timestamp state through the scripted frames so
// the sequence and ages read like a real feed.
type script struct {
	symbolID uint32
	seq      uint64
	tsNs     uint64
	rng      uint64
	frames   []any
}

// next advances the LCG and returns a value in [0, n).
func (s *script) next(n int64) int64 {
	s.rng = s.rng*6364136223846793005 + 1442695040888963407
	return int64((s.rng >> 33) % uint64(n))
}

// qty perturbs a base trade size by up to +/-10% off the seed, so a scene can
// vary without its structure moving.
func (s *script) qty(base int64) int64 { return base + s.next(base/5+1) - base/10 }

// tick advances the seq counter and the clock (~5ms per frame) and returns the
// pair to stamp on the next frame.
func (s *script) tick() (uint64, uint64) {
	s.seq++
	s.tsNs += 5_000_000
	return s.seq, s.tsNs
}

func (s *script) snapshot(bids, asks []wire.Level) {
	seq, ts := s.tick()
	s.frames = append(s.frames, wire.Snapshot{SymbolID: s.symbolID, Bids: bids, Asks: asks, TsNs: ts, Seq: seq})
}

func (s *script) delta(side uint32, px, qty int64, count uint32) {
	seq, ts := s.tick()
	s.frames = append(s.frames, wire.Delta{SymbolID: s.symbolID, Side: side, Px: px, Qty: qty, Count: count, TsNs: ts, Seq: seq})
}

func (s *script) bbo(bidPx, bidQty int64, bidCount uint32, askPx, askQty int64, askCount uint32) {
	seq, ts := s.tick()
	s.frames = append(s.frames, wire.Bbo{SymbolID: s.symbolID, BidPx: bidPx, BidQty: bidQty, BidCount: bidCount, AskPx: askPx, AskQty: askQty, AskCount: askCount, TsNs: ts, Seq: seq})
}

func (s *script) trade(px, qty int64, taker wire.Side) {
	seq, ts := s.tick()
	s.frames = append(s.frames, wire.MdTrade{SymbolID: s.symbolID, Px: px, Qty: qty, TakerSide: uint32(taker), TsNs: ts, Seq: seq})
}

func (s *script) heartbeat() {
	s.frames = append(s.frames, wire.MdHeartbeat{TsMs: s.tsNs / 1_000_000})
}
