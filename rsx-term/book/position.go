package book

import (
	"math"

	"rsx-term/wire"
)

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

// checkedAdd64 returns a+b and true, or (0, false) on signed-i64 overflow.
func checkedAdd64(a, b int64) (int64, bool) {
	sum := a + b
	if (b > 0 && sum < a) || (b < 0 && sum > a) {
		return 0, false
	}
	return sum, true
}

// checkedMul64 returns a*b and true, or (0, false) on signed-i64 overflow.
func checkedMul64(a, b int64) (int64, bool) {
	if a == 0 || b == 0 {
		return 0, true
	}
	p := a * b
	if p/b != a {
		return 0, false
	}
	return p, true
}

// checkedSub64 returns a-b and true, or (0, false) on signed-i64 overflow
// (including the math.MinInt64 negation edge case checkedAdd64(a, -b) alone
// would miss).
func checkedSub64(a, b int64) (int64, bool) {
	if b == math.MinInt64 {
		return 0, false
	}
	return checkedAdd64(a, -b)
}

// ApplyFill folds one own-account fill into the position. It reports false
// and leaves the position unchanged if any step would overflow i64 — an
// oversized fill is rejected rather than silently wrapping Net/Cost into a
// plausible-but-false figure (CLAUDE.md: check overflow with checked_mul at
// the boundary; POSITION-I64-OVERFLOW-WRAPS-PNL).
func (p *Position) ApplyFill(side wire.Side, px, qty int64) bool {
	signed := qty
	if side == wire.Sell {
		signed = -qty
	}

	if p.Net == 0 || sameSign(p.Net, signed) {
		notional, ok := checkedMul64(px, signed)
		if !ok {
			return false
		}
		net, ok := checkedAdd64(p.Net, signed)
		if !ok {
			return false
		}
		cost, ok := checkedAdd64(p.Cost, notional)
		if !ok {
			return false
		}
		p.Net, p.Cost = net, cost
		return true
	}

	prevAbs := abs64(p.Net)
	r := min64(abs64(signed), prevAbs)
	// Integer division truncates toward zero on every partial reduce, so cost
	// basis drifts down by up to 1 unit per reduce (Entry reads slightly
	// favorable over a long series of small closes). Acceptable for a display
	// figure; prevAbs is non-zero here since this branch only runs when Net != 0.
	scaled, ok := checkedMul64(p.Cost, prevAbs-r)
	if !ok {
		return false
	}
	newCost := scaled / prevAbs
	newNet, ok := checkedAdd64(p.Net, signed)
	if !ok {
		return false
	}

	if newNet == 0 {
		newCost = 0
	} else if abs64(signed) > r {
		flip, ok := checkedMul64(px, newNet)
		if !ok {
			return false
		}
		newCost = flip
	}
	p.Net, p.Cost = newNet, newCost
	return true
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

// Upnl returns Net*mark - Cost, or false when flat OR when the computation
// would overflow i64 — an unreliable figure is withheld rather than shown
// wrapped (POSITION-I64-OVERFLOW-WRAPS-PNL).
func (p Position) Upnl(mark int64) (int64, bool) {
	if p.Net == 0 {
		return 0, false
	}
	notional, ok := checkedMul64(p.Net, mark)
	if !ok {
		return 0, false
	}
	pnl, ok := checkedSub64(notional, p.Cost)
	if !ok {
		return 0, false
	}
	return pnl, true
}

// Flat reports whether the position is closed.
func (p Position) Flat() bool {
	return p.Net == 0
}
