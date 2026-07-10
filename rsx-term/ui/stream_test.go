package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/wire"
)

// streamModel builds a stream-mode model sized to a known terminal.
func streamModel(t *testing.T) Model {
	t.Helper()
	m := New(Config{Symbol: "PENGU-PERP", Stream: true, PriceDec: 6, QtyDec: 4, Tick: 1})
	next, _ := m.Update(tea.WindowSizeMsg{Width: 60, Height: 24})
	return next.(Model)
}

func TestDefaultViewUnchanged(t *testing.T) {
	// Stream OFF renders the classic DOM view (its book panel + help legend).
	dom := stripANSI(New(Config{Symbol: "PENGU-PERP"}).View())
	if !strings.Contains(dom, "book") || !strings.Contains(dom, "q quit  b/s side") {
		t.Fatalf("default view should be the DOM view: %q", dom)
	}
	if strings.Contains(dom, "streaming heatmap") {
		t.Fatalf("default view must not render the stream legend")
	}
	// Stream ON (sized) renders the heatmap header + legend, never the DOM panel.
	str := stripANSI(streamModel(t).View())
	if !strings.Contains(str, "mid") || !strings.Contains(str, "streaming heatmap") {
		t.Fatalf("stream view should render the heatmap: %q", str)
	}
}

func TestSizeTierLogScaled(t *testing.T) {
	if got := sizeTier(0, 1000); got != 0 {
		t.Fatalf("zero size => tier 0, got %d", got)
	}
	if got := sizeTier(1, 1000); got < 1 {
		t.Fatalf("any nonzero size => at least tier 1, got %d", got)
	}
	small := sizeTier(5, 1000)
	big := sizeTier(1000, 1000)
	if big <= small {
		t.Fatalf("bigger size => higher tier: small %d, big %d", small, big)
	}
	if big != sizeTiers {
		t.Fatalf("the reference max should hit the top tier, got %d", big)
	}
}

func TestCountTierWhaleVsWall(t *testing.T) {
	whale := countTier(1)
	wall := countTier(20)
	if whale >= wall {
		t.Fatalf("one whale should read fainter than a wall: whale %d, wall %d", whale, wall)
	}
	if shades[whale] == shades[wall] {
		t.Fatalf("whale and wall must pick different glyphs")
	}
}

func TestCellEncodingChannelsIndependent(t *testing.T) {
	// True-colour cell: background = size (log), glyph = order count. A whale
	// (huge size, one order) shows a faint ░; a retail wall (small size, many
	// orders) shows a solid █ — the two channels move independently.
	whale := book.Cell{Size: 1000, Count: 1, Side: -1}
	wall := book.Cell{Size: 10, Count: 20, Side: -1}
	if !strings.ContainsRune(cellStr(whale, 1000, 1, modeTrue), '░') {
		t.Fatalf("whale (count 1) should render the ░ glyph")
	}
	if !strings.ContainsRune(cellStr(wall, 1000, 1, modeTrue), '█') {
		t.Fatalf("wall (count 20) should render the █ glyph")
	}
}

func TestCellPlainDegrade(t *testing.T) {
	// Plain mode emits no ANSI and encodes size via the glyph (single channel).
	c := book.Cell{Size: 500, Count: 3, Side: 1}
	s := cellStr(c, 1000, 1, modePlain)
	if strings.Contains(s, "\x1b") {
		t.Fatalf("plain mode must emit no colour escapes: %q", s)
	}
	if s != string(shades[sizeTier(500, 1000)]) {
		t.Fatalf("plain glyph should encode the size tier: %q", s)
	}
}

func TestCellTradeOverlay(t *testing.T) {
	c := book.Cell{Size: 100, Count: 2, Side: -1, SellTrade: 9}
	if !strings.ContainsRune(stripANSI(cellStr(c, 1000, 1, modeTrue)), tradeGlyph) {
		t.Fatalf("a bin with a trade should overlay the trade glyph")
	}
}

func TestStreamFooterShowsTouch(t *testing.T) {
	m := streamModel(t)
	snap := wire.Snapshot{
		Bids: []wire.Level{{Px: 9999, Qty: 5, Count: 1}},
		Asks: []wire.Level{{Px: 10001, Qty: 6, Count: 1}},
		Seq:  1,
	}
	next, _ := m.Update(snap)
	m = next.(Model)
	foot := stripANSI(m.streamTouchLine())
	if !strings.Contains(foot, m.fmtPx(9999)) || !strings.Contains(foot, m.fmtPx(10001)) {
		t.Fatalf("touch line should show both sides at display precision: %q", foot)
	}
	if !strings.Contains(foot, "spread 2") {
		t.Fatalf("touch line should show the spread: %q", foot)
	}
}

func TestViewStreamRendersRing(t *testing.T) {
	m := streamModel(t)
	snap := wire.Snapshot{
		Bids: []wire.Level{{Px: 9999, Qty: 5, Count: 1}, {Px: 9998, Qty: 8, Count: 3}},
		Asks: []wire.Level{{Px: 10001, Qty: 6, Count: 1}, {Px: 10002, Qty: 4, Count: 2}},
		Seq:  1,
	}
	next, _ := m.Update(snap)
	m = next.(Model)
	// A trade this bin should be captured and overlaid.
	tr, _ := m.Update(wire.MdTrade{Px: 10001, Qty: 3, TakerSide: 0, Seq: 2})
	m = tr.(Model)
	for i := 0; i < 3; i++ {
		bt, _ := m.Update(binTickMsg(time.Now()))
		m = bt.(Model)
	}

	out := m.View()
	plain := stripANSI(out)
	for _, want := range []string{"RSX", "mid", "bid", "ask", "⚡", "assistant", "streaming heatmap"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("stream view missing %q:\n%s", want, plain)
		}
	}
	// Full screen = header (1) + body (heat height) + footer (5).
	wantLines := 1 + m.heat.Height() + 5
	if got := strings.Count(out, "\n") + 1; got != wantLines {
		t.Fatalf("stream view = %d lines, want %d", got, wantLines)
	}
}
