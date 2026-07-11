package ui

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/wire"
)

// demoSnapshot is a small live book for dispatch tests.
func demoSnapshot() wire.Snapshot {
	return wire.Snapshot{
		SymbolID: 10,
		Bids:     []wire.Level{{Px: 9999, Qty: 50000, Count: 1}},
		Asks:     []wire.Level{{Px: 10001, Qty: 60000, Count: 1}},
		Seq:      1,
	}
}

func TestKeymapDefaultLookup(t *testing.T) {
	k := defaultKeymap()
	cases := []struct {
		screen screen
		key    string
		want   action
	}{
		{screenBook, "f", actPlace},
		{screenBook, "left", actCursorDown}, // alternate key
		{screenBook, "q", actQuit},          // global falls through
		{screenBook, "b", actBuySide},       // screen verb
		{screenNews, "/", actSearch},
		{screenNews, "enter", actHandoff},
		{screenLLM, "esc", actBack},
		{screenBook, "7", actNone}, // key classes are not table verbs
	}
	for _, c := range cases {
		if got := k.lookup(c.screen, c.key); got != c.want {
			t.Fatalf("lookup(%s, %q) = %q, want %q", c.screen.label(), c.key, got, c.want)
		}
	}
}

func TestKeymapNoDefaultConflicts(t *testing.T) {
	if err := defaultKeymap().checkConflicts(); err != nil {
		t.Fatalf("shipped keymap conflicts: %v", err)
	}
}

func TestKeymapRebindsVerb(t *testing.T) {
	k := defaultKeymap()
	if err := k.ApplyOverrides(map[string]string{"place": "o"}); err != nil {
		t.Fatalf("rebind: %v", err)
	}
	if got := k.lookup(screenBook, "o"); got != actPlace {
		t.Fatalf("o should now place, got %q", got)
	}
	if got := k.lookup(screenBook, "f"); got != actNone {
		t.Fatalf("f should be unbound after the rebind, got %q", got)
	}
	if !strings.Contains(k.hintFor(screenBook), "o place") {
		t.Fatalf("hint line should show the rebound key: %q", k.hintFor(screenBook))
	}
}

func TestKeymapRejectsUnknownAndConflicts(t *testing.T) {
	if err := defaultKeymap().ApplyOverrides(map[string]string{"warp-speed": "w"}); err == nil {
		t.Fatalf("unknown action must be rejected")
	}
	// Rebinding place onto d collides with cancel on the book screen.
	if err := defaultKeymap().ApplyOverrides(map[string]string{"place": "d"}); err == nil {
		t.Fatalf("a key collision must be rejected")
	}
}

func TestKeymapOverrideDrivesDispatch(t *testing.T) {
	mock := &conn.MockGateway{}
	m := New(Config{
		Symbol: "PENGU-PERP", SymbolID: 10, Sub: mock,
		PriceDec: 6, QtyDec: 4, Tick: 1, Stream: true,
		KeyOverrides: map[string]string{"place": "o"},
	})
	m = apply(m, tea.WindowSizeMsg{Width: 100, Height: 30})
	m = apply(m, demoSnapshot())
	m = press(m, "o")
	if len(mock.Submitted) != 1 {
		t.Fatalf("the rebound key should place an order: %+v", mock.Submitted)
	}
	m = press(m, "f")
	if len(mock.Submitted) != 1 {
		t.Fatalf("the default key must be unbound after the rebind")
	}
}

func TestKeymapBrokenOverridesFallBackLoudly(t *testing.T) {
	m := New(Config{
		Symbol: "PENGU-PERP", SymbolID: 10, Sub: &conn.MockGateway{},
		Stream:       true,
		KeyOverrides: map[string]string{"nonsense": "z"},
	})
	if !strings.Contains(m.status, "KEYMAP REJECTED") {
		t.Fatalf("a broken keymap must be flagged: %q", m.status)
	}
	if m.keys.lookup(screenBook, "f") != actPlace {
		t.Fatalf("defaults must stay active after a rejected keymap")
	}
}

func TestSearchCapturesGlobalKeys(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "/")
	m = press(m, "q") // must TYPE q, not quit
	if m.newsQuery != "q" || m.screen != screenNews {
		t.Fatalf("search must capture q: query %q", m.newsQuery)
	}
}
