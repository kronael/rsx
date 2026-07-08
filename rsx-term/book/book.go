// Package book is pure state folding for the terminal: the L2 ladder,
// sequence-gap tracking, public trade tape, client-derived position, and
// latency samples. No I/O. See specs/2/55-terminal.md and
// specs/2/49-webproto.md.
package book

import "rsx-term/wire"

// Book is the L2 ladder for one symbol: descending bids, ascending asks,
// plus the last-seen BBO.
type Book struct {
	Bids []wire.Level
	Asks []wire.Level
	bbo  *wire.Bbo
}

// Bbo returns the last recorded BBO, or false if none has been applied.
func (b *Book) Bbo() (wire.Bbo, bool) {
	if b.bbo == nil {
		return wire.Bbo{}, false
	}
	return *b.bbo, true
}

// ApplySnapshot replaces both sides of the ladder wholesale. The wire order
// is trusted (bids descending, asks ascending) — no re-sort.
func (b *Book) ApplySnapshot(s wire.Snapshot) {
	b.Bids = s.Bids
	b.Asks = s.Asks
}

// ApplyBbo records the latest BBO.
func (b *Book) ApplyBbo(bb wire.Bbo) {
	cp := bb
	b.bbo = &cp
}

// ApplyDelta folds a single-level L2 update into the ladder. Side 0 = bid
// (kept descending by Px), side 1 = ask (kept ascending by Px). Qty == 0
// removes the level (no-op if absent); otherwise the level is inserted or
// replaced in place, preserving sort order. Any other side value is
// ignored.
func (b *Book) ApplyDelta(d wire.Delta) {
	switch d.Side {
	case 0:
		b.Bids = applyLevel(b.Bids, d, descending)
	case 1:
		b.Asks = applyLevel(b.Asks, d, ascending)
	}
}

type ordering int

const (
	descending ordering = iota
	ascending
)

// applyLevel inserts, replaces, or removes a level in a sorted slice.
func applyLevel(levels []wire.Level, d wire.Delta, order ordering) []wire.Level {
	idx := 0
	for idx < len(levels) {
		if levels[idx].Px == d.Px {
			break
		}
		if order == descending && levels[idx].Px < d.Px {
			break
		}
		if order == ascending && levels[idx].Px > d.Px {
			break
		}
		idx++
	}

	found := idx < len(levels) && levels[idx].Px == d.Px

	if d.Qty == 0 {
		if !found {
			return levels
		}
		return append(levels[:idx], levels[idx+1:]...)
	}

	lvl := wire.Level{Px: d.Px, Qty: d.Qty, Count: d.Count}
	if found {
		levels[idx] = lvl
		return levels
	}
	levels = append(levels, wire.Level{})
	copy(levels[idx+1:], levels[idx:])
	levels[idx] = lvl
	return levels
}

// BestBid returns the top of the bid ladder, or false if empty.
func (b *Book) BestBid() (wire.Level, bool) {
	if len(b.Bids) == 0 {
		return wire.Level{}, false
	}
	return b.Bids[0], true
}

// BestAsk returns the top of the ask ladder, or false if empty.
func (b *Book) BestAsk() (wire.Level, bool) {
	if len(b.Asks) == 0 {
		return wire.Level{}, false
	}
	return b.Asks[0], true
}

// Spread is best ask Px minus best bid Px, or 0 if either side is empty.
func (b *Book) Spread() int64 {
	bid, ok := b.BestBid()
	if !ok {
		return 0
	}
	ask, ok := b.BestAsk()
	if !ok {
		return 0
	}
	return ask.Px - bid.Px
}

// Mid returns the midpoint. It prefers the ladder (both sides present);
// if the ladder is missing a side it falls back to the recorded BBO, but
// only when both BBO sides are set (Px > 0 — proto3 zero means unset).
// Never fabricates a mid from a single side.
func (b *Book) Mid() (int64, bool) {
	bid, bidOk := b.BestBid()
	ask, askOk := b.BestAsk()
	if bidOk && askOk {
		return (bid.Px + ask.Px) / 2, true
	}
	if b.bbo != nil && b.bbo.BidPx > 0 && b.bbo.AskPx > 0 {
		return (b.bbo.BidPx + b.bbo.AskPx) / 2, true
	}
	return 0, false
}

// Empty reports whether the ladder has no bids and no asks.
func (b *Book) Empty() bool {
	return len(b.Bids) == 0 && len(b.Asks) == 0
}
