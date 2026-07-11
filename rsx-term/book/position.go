package book

import "rsx-term/wire"

// Position is a client-derived position, folded from the account's own
// fills only (P/A queries against the exchange are Post-MVP). Net is
// signed (+long/-short); Cost is the signed cost basis of the open
// position (buys add Px*Qty, so a short position ends up with negative
// Cost). uPnL derived from this position is labelled mark=mid in the UI
// and is never presented as an authoritative exchange figure.
type Position struct {
	Net  int64
	Cost int64
}

func abs64(v int64) int64 {
	if v < 0 {
		return -v
	}
	return v
}

func min64(a, b int64) int64 {
	if a < b {
		return a
	}
	return b
}

// ApplyFill folds one own-account fill into the position.
func (p *Position) ApplyFill(side wire.Side, px, qty int64) {
	signed := qty
	if side == wire.Sell {
		signed = -qty
	}

	if p.Net == 0 || sameSign(p.Net, signed) {
		p.Net += signed
		p.Cost += px * signed
		return
	}

	r := min64(abs64(signed), abs64(p.Net))
	prevAbs := abs64(p.Net)
	// Integer division truncates toward zero on every partial reduce, so cost
	// basis drifts down by up to 1 unit per reduce (Entry reads slightly
	// favorable over a long series of small closes). Acceptable for a display
	// figure; prevAbs is non-zero here since this branch only runs when Net != 0.
	p.Cost = p.Cost * (prevAbs - r) / prevAbs
	p.Net += signed

	if p.Net == 0 {
		p.Cost = 0
	} else if abs64(signed) > r {
		p.Cost = px * p.Net
	}
}

func sameSign(a, b int64) bool {
	return (a > 0 && b > 0) || (a < 0 && b < 0)
}

// Entry returns the average entry price (Cost/Net), or false when flat.
func (p Position) Entry() (int64, bool) {
	if p.Net == 0 {
		return 0, false
	}
	return p.Cost / p.Net, true
}

// Upnl returns Net*mark - Cost, or false when flat.
func (p Position) Upnl(mark int64) (int64, bool) {
	if p.Net == 0 {
		return 0, false
	}
	return p.Net*mark - p.Cost, true
}

// Flat reports whether the position is closed.
func (p Position) Flat() bool {
	return p.Net == 0
}
