// Package news is an optional headline feed for the streaming terminal. A
// Source returns time-stamped Markers that the heatmap overlays on its time
// axis (the left news rail). Every source defaults OFF so the terminal always
// runs offline; enabling a live source is an explicit opt-in and must never
// block startup or the render — a live source serves from an in-memory buffer
// filled by a background reader, returning whatever it has (possibly nothing).
package news

// Marker is one headline pinned to a point in time.
type Marker struct {
	// TsNs is the headline's wall-clock time (Unix epoch nanoseconds), matched
	// against a heatmap row's window to place the rail marker.
	TsNs int64
	// Text is the one-line headline.
	Text string
	// Source names the feed's origin tag (e.g. "Twitter", "Blogs").
	Source string
	// Symbols are the feed's symbol tags (upper-case, possibly pair-suffixed
	// like BTCUSDT), for cross-linking a headline to a market.
	Symbols []string
	// Tier ranks severity 0 (routine) … 3 (critical); the rail marker's
	// glyph/hue grades with it (progressive disclosure — the full headline
	// lives in the news view, never inline on the map).
	Tier int
}

// Source is a stream of news markers. Implementations must be non-blocking:
// Markers reads an in-memory buffer, never the network.
type Source interface {
	// Markers returns markers whose TsNs falls within [sinceNs, untilNs]. A
	// disabled source returns nil.
	Markers(sinceNs, untilNs int64) []Marker
	// Enabled reports whether the source is live. Off by default.
	Enabled() bool
}

// Off is the always-empty source and the prototype default: it connects to
// nothing and returns no markers, so the terminal renders fully offline.
type Off struct{}

// Markers always returns nil.
func (Off) Markers(_, _ int64) []Marker { return nil }

// Enabled always returns false.
func (Off) Enabled() bool { return false }
