package news

import "rsx-term/wire"

// Origin names what produced an assistant handoff — a UI-agnostic tag so a
// future GPU/bitmap frontend can branch on it without knowing the terminal.
type Origin int

const (
	// OriginNews is a headline the trader selected in the news feed.
	OriginNews Origin = iota
	// OriginBookFreeze is a heatmap row the trader froze under the book
	// microscope (a live ~100ms bin or an aggregate time-weighted window).
	OriginBookFreeze
)

// Label is the origin's human tag.
func (o Origin) Label() string {
	switch o {
	case OriginBookFreeze:
		return "book freeze"
	default:
		return "news"
	}
}

// AssistantContext is the GENERIC handoff the assistant pane receives: the
// market it concerns, when, an optional headline, an origin-specific note, and
// the frozen level set at handoff. It is a plain UI-agnostic struct (only wire
// + primitives — no terminal types), so the packaging (and its tests) outlive
// the placeholder assistant and a future GPU/bitmap frontend reuses it.
//
// The Headline is OPTIONAL (present only for OriginNews) — a book freeze must
// not fake a news marker. Bids/Asks are the DEEP-COPIED frozen row/book, so
// the context stays a snapshot even as the live book keeps folding.
type AssistantContext struct {
	// Origin is what produced this handoff.
	Origin Origin
	// Venue / Symbol name the market.
	Venue  string
	Symbol string
	// TsNs is when the handoff happened (wall clock, Unix ns).
	TsNs int64
	// Headline is the selected news item — nil unless Origin is OriginNews.
	Headline *Marker
	// Note is an origin-specific honest label (e.g. a book freeze's window
	// description). Optional.
	Note string
	// Bids / Asks are the frozen level set — DEEP-COPIED.
	Bids []wire.Level
	Asks []wire.Level
	// MidPx is the anchored mid at handoff (0 = no live book / unknown).
	MidPx int64
}

// PackageNews freezes a NEWS handoff: the selected headline plus the linked
// market's book at selection time (levels deep-copied).
func PackageNews(venue, symbol string, tsNs int64, headline Marker, bids, asks []wire.Level, midPx int64) AssistantContext {
	h := headline
	return AssistantContext{
		Origin:   OriginNews,
		Venue:    venue,
		Symbol:   symbol,
		TsNs:     tsNs,
		Headline: &h,
		Bids:     cloneLevels(bids),
		Asks:     cloneLevels(asks),
		MidPx:    midPx,
	}
}

// PackageBookFreeze freezes a BOOK-microscope handoff: a heatmap row's levels
// (deep-copied) with an honest note (a live bin, or an aggregate window). No
// headline — a book freeze never fakes a news marker.
func PackageBookFreeze(venue, symbol string, tsNs int64, note string, bids, asks []wire.Level, midPx int64) AssistantContext {
	return AssistantContext{
		Origin: OriginBookFreeze,
		Venue:  venue,
		Symbol: symbol,
		TsNs:   tsNs,
		Note:   note,
		Bids:   cloneLevels(bids),
		Asks:   cloneLevels(asks),
		MidPx:  midPx,
	}
}

// cloneLevels deep-copies a level slice so the context stays frozen as the
// live book folds on.
func cloneLevels(src []wire.Level) []wire.Level {
	if src == nil {
		return nil
	}
	return append([]wire.Level(nil), src...)
}
