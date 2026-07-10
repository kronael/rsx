package book

import "rsx-term/wire"

// AgeSource supplies the age of the resting liquidity at a price — the seam
// between the heatmap's persistence channel and whatever measures it. Today
// the only implementation is Persistence (below), a best-effort client-side
// proxy; when the exchange grows a real order-age feed (market-by-order, or a
// per-level age field in the marketdata protocol — an engine-side follow-up,
// out of scope here) it implements this same interface and the UI needs no
// rewrite.
type AgeSource interface {
	// AgeNs returns how long the level at px has persisted as of nowNs,
	// or 0 when the level is untracked/fresh.
	AgeNs(px int64, nowNs int64) int64
}

// Persistence tracks L2 level-persistence: how long each price level has
// held resting size across marketdata updates. It is a PROXY for order age —
// the L2 feed aggregates orders per level, so a level where fresh orders
// continually replace departing ones still reads as "standing". Accurate
// per-order longevity needs exchange support (see AgeSource). A level's
// clock starts when its price first shows size and resets only when the
// level empties.
type Persistence struct {
	since map[int64]int64 // px → ns the level became (and stayed) non-empty
}

// NewPersistence builds an empty tracker.
func NewPersistence() *Persistence {
	return &Persistence{since: make(map[int64]int64)}
}

// ObserveSnapshot folds a wholesale book replacement: levels present keep (or
// start) their clocks; tracked levels absent from the snapshot reset.
func (p *Persistence) ObserveSnapshot(bids, asks []wire.Level, nowNs int64) {
	alive := make(map[int64]bool, len(bids)+len(asks))
	for _, l := range bids {
		if l.Qty > 0 {
			alive[l.Px] = true
		}
	}
	for _, l := range asks {
		if l.Qty > 0 {
			alive[l.Px] = true
		}
	}
	for px := range p.since {
		if !alive[px] {
			delete(p.since, px)
		}
	}
	for px := range alive {
		if _, ok := p.since[px]; !ok {
			p.since[px] = nowNs
		}
	}
}

// ObserveDelta folds a single-level update: size at a new price starts its
// clock, an emptied level resets it, a size change on a standing level keeps
// it (the level is still held).
func (p *Persistence) ObserveDelta(d wire.Delta, nowNs int64) {
	if d.Qty <= 0 {
		delete(p.since, d.Px)
		return
	}
	if _, ok := p.since[d.Px]; !ok {
		p.since[d.Px] = nowNs
	}
}

// AgeNs satisfies AgeSource.
func (p *Persistence) AgeNs(px int64, nowNs int64) int64 {
	since, ok := p.since[px]
	if !ok {
		return 0
	}
	age := nowNs - since
	if age < 0 {
		return 0
	}
	return age
}
