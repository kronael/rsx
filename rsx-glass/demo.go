package glass

import (
	"rsx-core/book"
	"rsx-core/wire"
)

// Synth is a deterministic synthetic market: a mid that walks, a bid and an ask
// wall, and a trickle of trades — enough to drive the whole heatmap (mid band,
// walls, trades, persistence) without a live feed. It exists so the viewer runs
// standalone and so the goldens have a reproducible scene; wiring a real feed
// (rsx-term's conn, or the marketdata subscriber) is the v2 seam. The PRNG is a
// plain LCG, so the sequence is identical on every machine and Go version.
type Synth struct {
	heat      *book.Heatmap
	persist   *book.Persistence
	rng       uint64
	mid       int64
	nowNs     int64
	bids      []wire.Level
	asks      []wire.Level
	trades    []book.TapeEntry
	Connected bool
	Symbol    string
}

// NewSynth builds a synthetic feed whose heatmap is sized to a w×h grid.
func NewSynth(w, h int) *Synth {
	body := h - 4          // header(1) + body + ruler(1) + footer(2)
	far := 6               // aggregated far rows
	live := body - 1 - far // the rest are live bins; NOW is drawn separately
	if live < 1 {
		live = 1
	}
	heat := book.NewHeatmap(1)
	heat.Configure(live, far)
	return &Synth{
		heat:      heat,
		persist:   book.NewPersistence(),
		rng:       0x9e3779b97f4a7c15,
		mid:       10000,
		Connected: true,
		Symbol:    "BTC-PERP",
	}
}

// next returns a deterministic pseudo-random value in [0, n).
func (s *Synth) next(n int64) int64 {
	s.rng = s.rng*6364136223846793005 + 1442695040888963407
	return int64((s.rng >> 33) % uint64(n))
}

// Advance steps the market one bin: it walks the mid, rebuilds the book with its
// walls, prints a couple of trades, and folds the bin into the reused heatmap.
// dtNs is the bin's wall-clock width (wide bins fill the far tiers quickly).
func (s *Synth) Advance(dtNs int64) {
	s.mid += s.next(5) - 2
	if s.mid < 50 {
		s.mid = 50
	}
	s.rebuild()
	from, to := s.nowNs, s.nowNs+dtNs
	s.persist.ObserveSnapshot(s.bids, s.asks, to)
	s.heat.Ingest(s.bids, s.asks, s.trades, s.persist, from, to)
	s.nowNs = to
}

// rebuild lays out the current book: a SPARSE, heavy-tailed depth profile (most
// levels empty — dark negative space — with occasional concentrations) plus a
// heavy wall a few ticks off each touch, and a few trades straddling the spread.
// The spread of sizes exercises the whole ramp, dark (thin) → bright (walls).
func (s *Synth) rebuild() {
	const depth = 22
	s.bids = s.bids[:0]
	s.asks = s.asks[:0]
	add := func(dst *[]wire.Level, px int64, wall bool) {
		var sz int64
		switch {
		case wall:
			sz = 180000 + s.next(60000)
		case s.next(10) < 4: // ~60% of levels are empty → negative space
			return
		case s.next(9) == 0: // a fat level
			sz = 40000 + s.next(70000)
		default: // a thin level
			sz = 1500 + s.next(9000)
		}
		*dst = append(*dst, wire.Level{Px: px, Qty: sz, Count: uint32(1 + s.next(8))})
	}
	for i := int64(0); i < depth; i++ {
		add(&s.bids, s.mid-1-i, i == 3)
		add(&s.asks, s.mid+1+i, i == 2)
	}
	s.trades = s.trades[:0]
	s.trades = append(s.trades,
		book.TapeEntry{Side: wire.Buy, Px: s.mid + 1, Qty: 8000 + s.next(45000)},
		book.TapeEntry{Side: wire.Sell, Px: s.mid - 1, Qty: 8000 + s.next(35000)},
	)
}

// Scene snapshots the current state for the Grid builder.
func (s *Synth) Scene() Scene {
	return Scene{
		Symbol:    s.Symbol,
		Connected: s.Connected,
		Heat:      s.heat,
		Bids:      s.bids,
		Asks:      s.asks,
		Trades:    s.trades,
		Ages:      s.persist,
		FromNs:    s.nowNs,
		ToNs:      s.nowNs,
	}
}
