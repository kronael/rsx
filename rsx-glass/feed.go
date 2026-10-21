package glass

import (
	"time"

	"rsx-core/book"
	"rsx-core/feed"
	"rsx-core/wire"
)

// binInterval is the live-bin cadence: every bin seals the current book into a
// heatmap row. Bins are 100 ms (specs/03-heatmap.md).
const binInterval = 100 * time.Millisecond

// Feed is the model-fold adapter — the "caller (a feed adapter) fills" that the
// Scene contract names. It folds the decoded market-data events a connector
// delivers (wire.Snapshot / wire.Delta / wire.Bbo / wire.MdTrade, plus the
// feed.MdUp/MdDown link transitions) into the shared rsx-core model — book.Book,
// book.Heatmap, book.Persistence — using the folds specs/13-contract-marketdata.md
// names, then snapshots that model as a Scene the Grid builder
// composes. No network, no clock: the caller supplies bin timestamps, so a test
// drives it deterministically and the live transport passes wall time.
//
// It is a single-owner value (one goroutine folds; no locks): the connector
// hands events to one folding loop, exactly as the terminal folds tea messages.
type Feed struct {
	symbol    string
	symbolID  uint32
	connected bool

	book    book.Book
	heat    *book.Heatmap
	persist *book.Persistence
	pending []book.TapeEntry // trades printed since the last sealed bin

	lastBinNs int64
	nowNs     int64

	sizeBasis  int64 // slow-decaying ramp references (book.FoldBasis, per bin)
	tradeBasis int64
}

// NewFeed builds a Feed for one symbol, sizing its heatmap to a grid h rows
// tall the way NewSynth does (heatmapCap), so a Scene composes to fit exactly.
// tick is the symbol's smallest price increment (<=0 -> 1).
func NewFeed(symbol string, symbolID uint32, tick int64, h int) *Feed {
	live, far := heatmapCap(h)
	heat := book.NewHeatmap(tick)
	heat.Configure(live, far)
	return &Feed{
		symbol:   symbol,
		symbolID: symbolID,
		heat:     heat,
		persist:  book.NewPersistence(),
	}
}

// Apply folds one connector event into the model as of nowNs. Market-data
// frames for another symbol are ignored (the mock/live feed may carry peers);
// link transitions flip the connected flag the header's link dot reads.
func (f *Feed) Apply(msg any, nowNs int64) {
	if nowNs > f.nowNs {
		f.nowNs = nowNs
	}
	switch v := msg.(type) {
	case feed.MdUp:
		f.connected = true
	case feed.MdDown:
		f.connected = false
	case wire.Snapshot:
		if !f.matches(v.SymbolID) {
			return
		}
		f.book.ApplySnapshot(v)
		f.persist.ObserveSnapshot(v.Bids, v.Asks, nowNs)
	case wire.Delta:
		if !f.matches(v.SymbolID) {
			return
		}
		f.book.ApplyDelta(v)
		f.persist.ObserveDelta(v, nowNs)
	case wire.Bbo:
		if !f.matches(v.SymbolID) {
			return
		}
		f.book.ApplyBbo(v)
	case wire.MdTrade:
		if !f.matches(v.SymbolID) {
			return
		}
		f.pending = append(f.pending, book.TapeEntry{Side: takerSide(v.TakerSide), Px: v.Px, Qty: v.Qty})
	}
}

// Bin seals the current book, its pending trades, and its level ages into one
// heatmap row covering [lastBinNs, nowNs] — the same Ingest ui/update.go's
// foldBinAt calls — then clears the pending trades. It is a no-op until the
// heatmap is sized. The first bin seeds its start one interval back.
func (f *Feed) Bin(nowNs int64) {
	if nowNs > f.nowNs {
		f.nowNs = nowNs
	}
	if f.heat.LiveCap() == 0 {
		return
	}
	if f.lastBinNs == 0 {
		f.lastBinNs = nowNs - int64(binInterval)
	}
	f.heat.Ingest(f.book.Bids, f.book.Asks, f.pending, f.persist, f.lastBinNs, nowNs)
	f.sizeBasis, f.tradeBasis = foldRampBases(f.sizeBasis, f.tradeBasis, f.book.Bids, f.book.Asks, f.pending)
	f.lastBinNs = nowNs
	f.pending = f.pending[:0]
}

// SetConnected sets the link state directly (the transport uses feed.MdUp /
// MdDown through Apply; this is for callers that track the link themselves).
func (f *Feed) SetConnected(up bool) { f.connected = up }

// Scene snapshots the current model for the Grid builder: the reused heatmap,
// the current book's exact ladders, the trades since the last bin (the NOW
// row's live prints), and the persistence age source.
func (f *Feed) Scene() Scene {
	return Scene{
		Symbol:     f.symbol,
		Connected:  f.connected,
		Heat:       f.heat,
		Bids:       f.book.Bids,
		Asks:       f.book.Asks,
		Trades:     f.pending,
		Ages:       f.persist,
		FromNs:     f.lastBinNs,
		ToNs:       f.nowNs,
		RefBasis:   f.sizeBasis,
		TradeBasis: f.tradeBasis,
	}
}

// matches reports whether a frame's symbol belongs to this feed (0 = the
// unspecified id older mocks send, folded as the primary symbol).
func (f *Feed) matches(id uint32) bool { return id == 0 || id == f.symbolID }

// takerSide maps a marketdata taker side (0 buy / 1 sell) to wire.Side — the
// same mapping ui/update.go uses.
func takerSide(v uint32) wire.Side {
	if v != 0 {
		return wire.Sell
	}
	return wire.Buy
}

// foldRampBases advances the stable size/trade ramp references one bin
// (book.FoldBasis): rise to the current book/trade max, else decay slowly. The
// Grid builder floors these by the live frame — the anti-strobe basis, shared by
// the live Feed and the Synth demo.
func foldRampBases(sizeBasis, tradeBasis int64, bids, asks []wire.Level, trades []book.TapeEntry) (int64, int64) {
	var maxLevel int64
	for _, level := range bids {
		if level.Qty > maxLevel {
			maxLevel = level.Qty
		}
	}
	for _, level := range asks {
		if level.Qty > maxLevel {
			maxLevel = level.Qty
		}
	}
	var maxTrade int64
	for _, t := range trades {
		if t.Qty > maxTrade {
			maxTrade = t.Qty
		}
	}
	return book.FoldBasis(sizeBasis, maxLevel), book.FoldBasis(tradeBasis, maxTrade)
}
