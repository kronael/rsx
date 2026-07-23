package book

import (
	"testing"
	"time"
)

// nsAt is a UTC wall-clock instant in Unix nanoseconds.
func nsAt(h, mi int) int64 {
	return time.Date(2026, 7, 23, h, mi, 0, 0, time.UTC).UnixNano()
}

func TestSessionMarkerContainsOpen(t *testing.T) {
	opens := DefaultSessionOpens()
	// A 1h window straddling 08:00 UTC catches the Europe open.
	if label, ok := SessionMarker(nsAt(7, 15), nsAt(8, 15), opens); !ok || label != "EU" {
		t.Fatalf("window over 08:00 = %q,%v, want EU,true", label, ok)
	}
	// The 14:30 US open lands in a window that spans it.
	if label, ok := SessionMarker(nsAt(14, 0), nsAt(15, 0), opens); !ok || label != "US" {
		t.Fatalf("window over 14:30 = %q,%v, want US,true", label, ok)
	}
	// A quiet hour with no default open reports nothing.
	if _, ok := SessionMarker(nsAt(2, 0), nsAt(3, 0), opens); ok {
		t.Fatalf("window with no open should not label")
	}
}

func TestSessionMarkerPriorityFirstInOrder(t *testing.T) {
	opens := DefaultSessionOpens()
	// A full day contains every open; priority order returns the first (day roll).
	from := nsAt(0, 0)
	if label, ok := SessionMarker(from, from+dayNs, opens); !ok || label != "day" {
		t.Fatalf("full-day window = %q,%v, want day,true (first in order)", label, ok)
	}
}

func TestSessionMarkerEmptyWindow(t *testing.T) {
	// An unfilled far tier snapshots with from==to==0 — no real window, no marker.
	if _, ok := SessionMarker(0, 0, DefaultSessionOpens()); ok {
		t.Fatalf("empty window must not label")
	}
}

func TestRowSessionOpenLabelGatesOnSpan(t *testing.T) {
	opens := DefaultSessionOpens()
	from, to := nsAt(7, 15), nsAt(8, 15) // spans the 08:00 EU open
	// A fine (sub-1h) far row never carries a session marker.
	fine := Row{Span: 60 * time.Second, FromNs: from, ToNs: to}
	if _, ok := fine.SessionOpenLabel(opens); ok {
		t.Fatalf("a sub-1h row must not carry a session marker")
	}
	// The 1h tier does.
	hour := Row{Span: time.Hour, FromNs: from, ToNs: to}
	if label, ok := hour.SessionOpenLabel(opens); !ok || label != "EU" {
		t.Fatalf("1h row = %q,%v, want EU,true", label, ok)
	}
	// A day row rolls the day marker.
	day := Row{Span: 24 * time.Hour, FromNs: nsAt(0, 0), ToNs: nsAt(0, 0) + dayNs}
	if label, ok := day.SessionOpenLabel(opens); !ok || label != "day" {
		t.Fatalf("day row = %q,%v, want day,true", label, ok)
	}
}

func TestParseSessionOpens(t *testing.T) {
	// Empty falls back to the four defaults.
	def, err := ParseSessionOpens("")
	if err != nil || len(def) != 4 || def[0].Label != "day" {
		t.Fatalf("empty spec = %+v,%v, want the 4 defaults", def, err)
	}
	// A custom spec preserves order as priority order.
	got, err := ParseSessionOpens("US=14:30,EU=08:00")
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if len(got) != 2 || got[0] != (SessionOpen{Label: "US", Hour: 14, Minute: 30}) ||
		got[1] != (SessionOpen{Label: "EU", Hour: 8, Minute: 0}) {
		t.Fatalf("parsed opens = %+v, want [US 14:30, EU 08:00] in order", got)
	}
	for _, bad := range []string{"US", "US=14", "US=99:00", "US=14:70", "=14:30"} {
		if _, err := ParseSessionOpens(bad); err == nil {
			t.Fatalf("malformed spec %q should error", bad)
		}
	}
}
