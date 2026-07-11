package main

import "sync"

// PriceSource yields a reference price for a symbol, or ok=false when
// it has no opinion. The maker quotes ref ± spread around whatever the
// first willing source returns.
type PriceSource interface {
	Ref(symbol uint32) (px int64, ok bool)
}

// composite tries each source in order and returns the first hit. Put
// the mark source first and the BBO-mid source second so a real index
// price is always preferred over the book mid.
type composite struct {
	sources []PriceSource
}

func newComposite(sources ...PriceSource) *composite {
	return &composite{sources: sources}
}

func (c *composite) Ref(symbol uint32) (int64, bool) {
	for _, s := range c.sources {
		if px, ok := s.Ref(symbol); ok {
			return px, true
		}
	}
	return 0, false
}

// bboSource tracks the live mid from the marketdata BBO feed. The
// marketdata reader calls update; the quote loop calls Ref.
type bboSource struct {
	mu   sync.RWMutex
	mids map[uint32]int64
}

func newBBOSource() *bboSource {
	return &bboSource{mids: make(map[uint32]int64)}
}

// update records a new mid from a BBO tick. Non-positive sides are
// ignored (an empty book side leaves the last good mid in place).
func (b *bboSource) update(symbol uint32, bidPx, askPx int64) {
	if bidPx <= 0 || askPx <= 0 {
		return
	}
	b.mu.Lock()
	b.mids[symbol] = (bidPx + askPx) / 2
	b.mu.Unlock()
}

func (b *bboSource) Ref(symbol uint32) (int64, bool) {
	b.mu.RLock()
	px, ok := b.mids[symbol]
	b.mu.RUnlock()
	return px, ok
}

// markSource is the seam for the true index/mark price. The cluster
// truth lives in RECORD_MARK_PRICE records on the separate `mark` WAL
// stream (rsx-mark writes it; the playground reads it via
// parse_wal_mark_prices). Consuming that stream means WAL/replication
// plumbing we deliberately do NOT build here — and the mark only
// becomes meaningful once an external feed anchors it (a later
// mirror-maker task). Until then this always abstains, so the
// composite falls through to the BBO mid.
//
// TODO(mark-wal): back this with a reader of the `mark` WAL stream
// (or rsx-mark's cast feed) so the maker quotes around the real index
// price instead of the book mid. Keep the PriceSource contract: return
// (px, true) only when a fresh mark exists for the symbol. NOTE: the
// Python maker's runtime only lets a fresh mark overwrite the mid while
// it is still the default — once live BBO has set a mid, BBO wins every
// cycle (despite its "mark > BBO" docstring). Match that behavior when
// filling this seam, not the composite's mark-first order, to keep demo
// parity.
type markSource struct{}

func newMarkSource() *markSource {
	return &markSource{}
}

func (m *markSource) Ref(symbol uint32) (int64, bool) {
	return 0, false
}
