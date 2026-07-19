package glass

import (
	"time"

	"rsx-term/book"
	"rsx-term/feed"
	"rsx-core/wire"
)

// binInterval is the live-bin cadence: every bin seals the current book into a
// heatmap row. It matches rsx-term's streaming terminal (100 ms bins).
const binInterval = 100 * time.Millisecond

// Feed is the model-fold adapter — the "caller (a feed adapter) fills" that the
// Scene contract names. It folds the decoded market-data events a connector
// delivers (wire.Snapshot / wire.Delta / wire.Bbo / wire.MdTrade, plus the
// feed.MdUp/MdDown link transitions) into rsx-term's REUSED model — book.Book,
// book.Heatmap, book.Persistence — using the exact same folds the terminal's
// ui/update.go drives, then snapshots that model as a Scene the Grid builder
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
}

// NewFeed builds a Feed for one symbol, sizing its heatmap to a w x h grid the
// way NewSynth does (header 1 + body + ruler 1 + footer 2), so a Scene composes
// to fit exactly. tick is the symbol's smallest price increment (<=0 -> 1).
func NewFeed(symbol string, symbolID uint32, tick int64, w, h int) *Feed {
	body := h - 4
	far := 6
	live := body - 1 - far
	if live < 1 {
		live = 1
	}
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
		Symbol:    f.symbol,
		Connected: f.connected,
		Heat:      f.heat,
		Bids:      f.book.Bids,
		Asks:      f.book.Asks,
		Trades:    f.pending,
		Ages:      f.persist,
		FromNs:    f.lastBinNs,
		ToNs:      f.nowNs,
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
