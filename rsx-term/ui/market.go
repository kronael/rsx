package ui

import (
	"strings"

	"rsx-term/book"
	"rsx-term/wire"
)

// Instrument is one tradeable symbol's display/config surface: id, name, the
// switcher letter code, display precision, and tick. The gateway's /v1/symbols
// carries no names yet, so names come from the watch config (RSX_TERM_WATCH)
// with an honest SYM-<id> fallback.
type Instrument struct {
	ID       uint32
	Name     string
	Code     string
	PriceDec int
	QtyDec   int
	Tick     int64
	Sector   string // news-view market-map grouping ("" → "perps")
}

// market is one symbol's live state in the streaming terminal: its book,
// tape, heatmap, persistence tracker, position, and the pair-view flow/depth
// reads. All symbols on the watchlist fold in parallel; the active one is
// rendered.
type market struct {
	ins       Instrument
	book      book.Book
	tape      book.Tape
	heat      *book.Heatmap
	persist   *book.Persistence
	pending   []book.TapeEntry
	lastBinNs int64
	position  book.Position

	// Stable ramp references (rise instantly, decay slowly — see foldBasis).
	sizeBasis  int64
	tradeBasis int64

	// cursorPx is the depth-view price cursor (per symbol — hopping symbols
	// keeps each cursor where you left it).
	cursorPx int64

	// Pair-view reads: sparks is the recent mid per bin (ring, oldest first);
	// flowBuy/flowSell are decaying aggressor-flow accumulators; depthBasis
	// is the slow reference the depth-state glyph compares against.
	sparks     []int64
	flowBuy    int64
	flowSell   int64
	depthBasis int64
}

// sparkLen bounds the pair-view mid history (~6s of 100ms bins).
const sparkLen = 64

// newMarket builds an empty market for an instrument.
func newMarket(ins Instrument) *market {
	return &market{
		ins:        ins,
		heat:       book.NewHeatmap(ins.Tick),
		persist:    book.NewPersistence(),
		sizeBasis:  1,
		tradeBasis: 1,
		depthBasis: 1,
	}
}

// letterPool is the pair-view symbol-selector alphabet, home row first. The
// action keys stay OUT of it: b/s (verbs), d (cancel), r/p (modifier
// toggles), digits (counts), x (switcher), q (quit), and punctuation. ~20
// symbols get one letter each; past the pool, codes go two-letter.
const letterPool = "afghjkl" + "wetyuio" + "zcvnm"

// assignCodes gives every instrument a switcher/selector code: an explicit
// Code is kept (and its letters reserved); the rest draw single letters from
// letterPool — preferring the name's own first letter — then two-letter
// combinations. Deterministic in list order.
func assignCodes(instruments []Instrument) {
	used := map[string]bool{}
	for i := range instruments {
		if c := strings.ToLower(instruments[i].Code); c != "" {
			instruments[i].Code = c
			used[c] = true
		}
	}
	for i := range instruments {
		if instruments[i].Code != "" {
			continue
		}
		instruments[i].Code = pickCode(instruments[i].Name, used)
		used[instruments[i].Code] = true
	}
}

// pickCode chooses the first free code for a name: its own first pooled
// letter, any pool letter, then pool-letter pairs.
func pickCode(name string, used map[string]bool) string {
	lower := strings.ToLower(name)
	for _, r := range lower {
		if strings.ContainsRune(letterPool, r) && !used[string(r)] {
			return string(r)
		}
	}
	for _, r := range letterPool {
		if !used[string(r)] {
			return string(r)
		}
	}
	for _, a := range letterPool {
		for _, b := range letterPool {
			code := string(a) + string(b)
			if !used[code] {
				return code
			}
		}
	}
	return "??" // >600 symbols: beyond the terminal's design point
}

// watchlist is one named set of instruments on ONE venue that the pair view
// rotates through ([ / ] switch lists; letters select within the active
// list).
type watchlist struct {
	name  string
	venue string
	ids   []uint32
}

// foldPairReads advances a market's pair-view metrics at bin cadence: the mid
// spark ring, the decaying aggressor flow, and the slow depth basis.
func (mk *market) foldPairReads() {
	if mid, ok := mk.book.Mid(); ok {
		mk.sparks = append(mk.sparks, mid)
		if len(mk.sparks) > sparkLen {
			mk.sparks = mk.sparks[len(mk.sparks)-sparkLen:]
		}
	}
	for _, t := range mk.pending {
		if t.Side == wire.Sell {
			mk.flowSell += t.Qty
		} else {
			mk.flowBuy += t.Qty
		}
	}
	mk.flowBuy -= mk.flowBuy >> 5 // ~2s half-life at 100ms bins
	mk.flowSell -= mk.flowSell >> 5
	mk.depthBasis = foldBasis(mk.depthBasis, mk.totalDepth())
}

// moveBp is the mid's move vs the session reference (the oldest retained
// spark), in basis points. The news view's sector tiles read it.
func (mk *market) moveBp() (int64, bool) {
	mid, ok := mk.book.Mid()
	if !ok || len(mk.sparks) == 0 || mk.sparks[0] <= 0 {
		return 0, false
	}
	return (mid - mk.sparks[0]) * 10_000 / mk.sparks[0], true
}

// totalDepth is the whole visible book's resting size, both sides.
func (mk *market) totalDepth() int64 {
	return sideDepth(mk.book.Bids) + sideDepth(mk.book.Asks)
}

// sideDepth sums one side's visible resting size.
func sideDepth(levels []wire.Level) int64 {
	var total int64
	for _, l := range levels {
		total += l.Qty
	}
	return total
}
