package ui

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/conn"
	"rsx-term/feed"
	"rsx-term/wire"
)

func TestViewBasics(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	v := m.View()
	for _, want := range []string{"PENGU-PERP", "book", "positions (mark=mid)", "F3 trace"} {
		if !strings.Contains(v, want) {
			t.Fatalf("view missing %q", want)
		}
	}
}

func TestViewEmptyBookIsDegraded(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	if !strings.Contains(m.View(), "no live book") {
		t.Fatalf("empty book not shown as degraded")
	}
}

func TestViewWaitingBeforeLatency(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	if !strings.Contains(m.View(), "waiting") {
		t.Fatalf("speed strip does not show waiting state")
	}
}

func TestViewRttAfterLatency(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = apply(m, feed.Latency{Sample: book.Sample{TotalNs: 10440, NetNs: 2500, InternalNs: 7600, EngineNs: 340}})
	if !strings.Contains(m.View(), "RTT") {
		t.Fatalf("speed strip does not show RTT after a sample")
	}
}

func TestViewTrace(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = press(m, "f3")
	if !strings.Contains(m.View(), "TRACE") {
		t.Fatalf("f3 did not show TRACE panel")
	}
}

func TestViewTinyWindowDoesNotPanic(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = apply(m, tea.WindowSizeMsg{Width: 20, Height: 8})
	_ = m.View() // just must not panic
}

func TestViewLiveBookAndPosition(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	// Fold the full demo so the ladder + derived position render.
	m = apply(m, feed.MdUp{})
	m = apply(m, wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 10000, Qty: 7, Count: 1}, {Px: 9998, Qty: 9, Count: 1}},
		Asks:     []wire.Level{{Px: 10001, Qty: 5, Count: 1}},
		Seq:      1,
	})
	m = apply(m, wire.Fill{Oid: 7, Px: 9998, Qty: 14, Side: wire.Buy})
	v := m.View()
	if strings.Contains(v, "no live book") {
		t.Fatalf("live book still shows degraded row")
	}
	if !strings.Contains(v, "LONG") {
		t.Fatalf("position row missing LONG")
	}
}
