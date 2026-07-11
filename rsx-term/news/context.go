package news

import "rsx-term/wire"

// AssistantContext is the REAL handoff the LLM assistant pane receives when
// a trader selects a headline: the market it concerns, when, the headline
// itself, and the book frozen at handoff. The assistant is a placeholder
// today; this struct is not — it is the contract a future model call gets,
// so the packaging (and its tests) outlive the stub.
type AssistantContext struct {
	// Venue / Symbol name the market the headline was linked to.
	Venue  string
	Symbol string
	// TsNs is when the handoff happened (wall clock, Unix ns).
	TsNs int64
	// Headline is the selected news item.
	Headline Marker
	// Bids / Asks are the market's book at handoff — DEEP-COPIED, so the
	// context stays a snapshot even as the live book keeps folding.
	Bids []wire.Level
	Asks []wire.Level
	// MidPx is the anchored mid at handoff (0 = no live book).
	MidPx int64
}

// PackageContext freezes a handoff context (copies the level slices).
func PackageContext(venue, symbol string, tsNs int64, headline Marker, bids, asks []wire.Level, midPx int64) AssistantContext {
	return AssistantContext{
		Venue:    venue,
		Symbol:   symbol,
		TsNs:     tsNs,
		Headline: headline,
		Bids:     append([]wire.Level(nil), bids...),
		Asks:     append([]wire.Level(nil), asks...),
		MidPx:    midPx,
	}
}
