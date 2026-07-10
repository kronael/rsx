package news

// TreeOfAlphaURL is the Tree of Alpha news websocket endpoint. On connect the
// server streams JSON headline objects roughly shaped like:
//
//	{"title":"...","source":"Twitter","symbols":["BTC"],"time":1700000000000, ...}
//
// where "time" is Unix epoch milliseconds. A real reader would dial this URL,
// decode each frame into a Marker (TsNs = time*1e6), and append it to an
// in-memory ring; Markers would then serve from that ring. The prototype ships
// no live reader — see Start.
const TreeOfAlphaURL = "wss://news.treeofalpha.com/ws"

// TreeOfAlpha is a stub client for the Tree of Alpha news websocket. It is
// DISABLED by default and returns no markers, so nothing dials the network and
// the terminal always starts offline. Enabling a real feed is a deliberate,
// separate step (Start), never on the render or startup path.
type TreeOfAlpha struct {
	enabled bool
	// markers would be the reader goroutine's in-memory buffer. The prototype
	// never fills it.
	markers []Marker
}

// Start is the opt-in that a future build would call to bring the feed live: it
// would launch a named background reader that dials TreeOfAlphaURL and appends
// decoded headlines to t.markers. The prototype intentionally does NOT dial —
// it only flips the enabled flag so the plumbing is exercised without any
// network I/O. Nothing in the shipped terminal calls Start.
func (t *TreeOfAlpha) Start() {
	t.enabled = true
}

// Markers returns buffered markers within [sinceNs, untilNs]; empty while the
// stub carries no reader. Never blocks (in-memory only).
func (t *TreeOfAlpha) Markers(sinceNs, untilNs int64) []Marker {
	if !t.enabled {
		return nil
	}
	var out []Marker
	for _, m := range t.markers {
		if m.TsNs >= sinceNs && m.TsNs <= untilNs {
			out = append(out, m)
		}
	}
	return out
}

// Enabled reports whether Start has been called.
func (t *TreeOfAlpha) Enabled() bool { return t.enabled }
