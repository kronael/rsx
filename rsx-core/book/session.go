package book

import (
	"fmt"
	"strconv"
	"strings"
	"time"
)

// SessionOpen is a daily market-session boundary at a fixed UTC time-of-day —
// a wall-clock reference the log-time heatmap labels on its coarse (>= 1h) far
// rows so the trader can orient a day of aggregated liquidity ("this row spans
// the US open"). The defaults are the UTC day roll plus the Asia/Europe/US cash
// opens; they are configurable (DefaultSessionOpens / ParseSessionOpens)
// because a venue's meaningful boundaries differ. This is pure reference data —
// the model neither reads a clock nor renders; a consumer passes a row's
// wall-clock window and gets back a label.
type SessionOpen struct {
	Label  string
	Hour   int // UTC hour, 0..23
	Minute int // UTC minute, 0..59
}

// DefaultSessionOpens is the built-in schedule: the UTC day roll (00:00) and
// the Asia (01:00), Europe (08:00) and US (14:30 UTC = NY 09:30 ET) cash opens.
// The slice order is PRIORITY order — SessionMarker returns the first open
// whose instance lands in a row's window, so the day roll wins a full-day row
// while an intraday open labels the finer 1h row.
func DefaultSessionOpens() []SessionOpen {
	return []SessionOpen{
		{Label: "day", Hour: 0, Minute: 0},
		{Label: "Asia", Hour: 1, Minute: 0},
		{Label: "EU", Hour: 8, Minute: 0},
		{Label: "US", Hour: 14, Minute: 30},
	}
}

// dayNs is one UTC day in nanoseconds. UTC has no DST, so day boundaries are
// exact multiples of this from the Unix epoch (itself a 00:00 UTC day roll) —
// which is what lets SessionMarker do its window math in plain integers.
const dayNs = int64(24 * time.Hour)

// sessionTierMin is the minimum far-row span that carries a session-open
// marker: the coarse "day+" tiers (the 1h row and the stacked day rows), not
// the sub-minute live-adjacent tiers where an open would flicker in and out of
// a few-second window frame to frame.
const sessionTierMin = time.Hour

// SessionMarker returns the label of the first session open (in opens' priority
// order) whose daily UTC instance falls within the wall-clock window
// [fromNs, toNs). ok is false when no open lands in the window — including the
// empty from==to==0 window of an unfilled far tier. Pure integer arithmetic in
// Unix nanoseconds, so it never allocates on the render path.
func SessionMarker(fromNs, toNs int64, opens []SessionOpen) (string, bool) {
	for _, session := range opens {
		if session.inWindow(fromNs, toNs) {
			return session.Label, true
		}
	}
	return "", false
}

// inWindow reports whether this open's daily UTC recurrence has an instance in
// [fromNs, toNs): the first instance at or after fromNs, tested against toNs.
func (session SessionOpen) inWindow(fromNs, toNs int64) bool {
	if toNs <= fromNs || fromNs < 0 {
		return false
	}
	ofDay := int64(session.Hour)*int64(time.Hour) + int64(session.Minute)*int64(time.Minute)
	open := (fromNs/dayNs)*dayNs + ofDay
	if open < fromNs {
		open += dayNs
	}
	return open < toNs
}

// SessionOpenLabel is the session-open label to show in this row's time gutter,
// or ok=false to fall back to the plain horizon. It fires only on the coarse
// far tiers (Span >= 1h) whose wall-clock window contains an open; a live or
// fine far row never carries one. Every renderer calls
// this, so the "which rows, which open" rule lives in one place.
func (r Row) SessionOpenLabel(opens []SessionOpen) (string, bool) {
	if r.Span < sessionTierMin {
		return "", false
	}
	return SessionMarker(r.FromNs, r.ToNs, opens)
}

// ParseSessionOpens parses a config string of comma-separated `label=HH:MM` UTC
// opens (e.g. "day=00:00,Asia=01:00,EU=08:00,US=14:30"). An empty string yields
// DefaultSessionOpens; slice order is preserved as priority order. A malformed
// entry is a hard error — session config is startup config, not a hot-path
// input, so the caller fails loud rather than silently dropping a marker.
func ParseSessionOpens(spec string) ([]SessionOpen, error) {
	spec = strings.TrimSpace(spec)
	if spec == "" {
		return DefaultSessionOpens(), nil
	}
	var opens []SessionOpen
	for _, part := range strings.Split(spec, ",") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		label, clock, ok := strings.Cut(part, "=")
		if !ok {
			return nil, fmt.Errorf("session open %q: want label=HH:MM", part)
		}
		hh, mm, ok := strings.Cut(clock, ":")
		if !ok {
			return nil, fmt.Errorf("session open %q: want an HH:MM time", part)
		}
		h, err := strconv.Atoi(strings.TrimSpace(hh))
		if err != nil || h < 0 || h > 23 {
			return nil, fmt.Errorf("session open %q: hour must be 0..23", part)
		}
		m, err := strconv.Atoi(strings.TrimSpace(mm))
		if err != nil || m < 0 || m > 59 {
			return nil, fmt.Errorf("session open %q: minute must be 0..59", part)
		}
		label = strings.TrimSpace(label)
		if label == "" {
			return nil, fmt.Errorf("session open %q: empty label", part)
		}
		opens = append(opens, SessionOpen{Label: label, Hour: h, Minute: m})
	}
	if len(opens) == 0 {
		return DefaultSessionOpens(), nil
	}
	return opens, nil
}
