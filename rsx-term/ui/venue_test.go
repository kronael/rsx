package ui

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/feed"
	"rsx-term/wire"
)

// venueModel is a stream model with a primary rsx venue plus a read-only
// "hyperliquid" venue carrying one instrument.
func venueModel(t *testing.T, mock *conn.MockGateway) Model {
	t.Helper()
	m := New(Config{
		Symbol:   "PENGU-PERP",
		SymbolID: 10,
		Sub:      mock,
		PriceDec: 6,
		QtyDec:   4,
		Tick:     1,
		Stream:   true,
		Instruments: []Instrument{
			{ID: 10, Name: "PENGU-PERP", PriceDec: 6, QtyDec: 4, Tick: 1},
		},
		Venues: []VenueConfig{{
			Name: "hyperliquid",
			Code: "h",
			Instruments: []Instrument{
				{ID: 1, Name: "BTC", PriceDec: 1, QtyDec: 5, Tick: 1},
			},
		}},
	})
	return apply(m, tea.WindowSizeMsg{Width: 100, Height: 30})
}

func TestVenueMsgRoutesToItsOwnMarket(t *testing.T) {
	m := venueModel(t, &conn.MockGateway{})
	m = apply(m, feed.VenueMsg{Venue: "hyperliquid", Msg: wire.Snapshot{
		SymbolID: 1,
		Bids:     []wire.Level{{Px: 429_990, Qty: 5, Count: 1}},
		Asks:     []wire.Level{{Px: 430_010, Qty: 5, Count: 1}},
	}})
	if got := m.marketFor("hyperliquid", 1).book.Spread(); got != 20 {
		t.Fatalf("hl market should hold the tagged book: spread %d", got)
	}
	if !m.book.Empty() {
		t.Fatalf("a tagged venue frame must NOT touch the primary/DOM book")
	}
	if got := m.marketFor("rsx", 10).book.Spread(); got != 0 {
		t.Fatalf("rsx market must stay empty: %d", got)
	}
}

func TestVenuePickerSwitchesBook(t *testing.T) {
	m := venueModel(t, &conn.MockGateway{})
	m = press(m, "f9")
	if !m.venuePicking {
		t.Fatalf("F9 should open the venue picker")
	}
	m = press(m, "h")
	if m.activeVenue != "hyperliquid" || m.active != 1 {
		t.Fatalf("picker should land on hl's first instrument: %s/%d", m.activeVenue, m.active)
	}
	if !strings.Contains(stripANSI(m.View()), "BTC") {
		t.Fatalf("book header should show the hl instrument")
	}
}

func TestReadOnlyVenueBlocksOrders(t *testing.T) {
	mock := &conn.MockGateway{}
	m := venueModel(t, mock)
	m = press(m, "f9")
	m = press(m, "h")
	m = apply(m, feed.VenueMsg{Venue: "hyperliquid", Msg: wire.Snapshot{
		SymbolID: 1,
		Bids:     []wire.Level{{Px: 429_990, Qty: 5, Count: 1}},
		Asks:     []wire.Level{{Px: 430_010, Qty: 5, Count: 1}},
	}})
	m = press(m, "!") // aggressive cross on a read-only venue
	if len(mock.Submitted) != 0 {
		t.Fatalf("read-only venue must never submit: %+v", mock.Submitted)
	}
	if !strings.Contains(m.status, "read-only") {
		t.Fatalf("status should explain the block: %q", m.status)
	}
}

func TestPairDefaultsToBreadthVenue(t *testing.T) {
	m := venueModel(t, &conn.MockGateway{})
	if got := m.pairVenue(); got != "hyperliquid" {
		t.Fatalf("pair list should default to the breadth venue, got %q", got)
	}
	m = press(m, "tab") // pair screen
	if !strings.Contains(stripANSI(m.View()), "BTC") {
		t.Fatalf("pair view should list the hl universe")
	}
}
