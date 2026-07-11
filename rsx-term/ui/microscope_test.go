package ui

import (
	"strings"
	"testing"

	"rsx-term/conn"
	"rsx-term/news"
)

func TestMicroscopeEnterMoveExit(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	if m.rowCursor != -1 {
		t.Fatalf("microscope must start off, got rowCursor %d", m.rowCursor)
	}
	n := len(m.mkt().heat.Rows())
	m = press(m, "up") // first arrow enters at the newest held row
	if m.rowCursor != n-1 {
		t.Fatalf("first up enters at the newest row %d, got %d", n-1, m.rowCursor)
	}
	for i := 0; i < n+2; i++ {
		m = press(m, "up") // walk up, then clamp at the oldest row
	}
	if m.rowCursor != 0 {
		t.Fatalf("up must clamp at the oldest row 0, got %d", m.rowCursor)
	}
	m = press(m, "down")
	if m.rowCursor != 1 {
		t.Fatalf("down should step toward newer rows, got %d", m.rowCursor)
	}
	m = press(m, "esc") // esc steps OUT of the microscope (does not quit)
	if m.rowCursor != -1 || m.screen != screenBook {
		t.Fatalf("esc should turn the microscope off: rowCursor %d screen %v", m.rowCursor, m.screen)
	}
}

func TestMicroscopeFarRowIsHonestWindow(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	n := len(m.mkt().heat.Rows())
	for i := 0; i < n+2; i++ {
		m = press(m, "up") // enter, then walk into the far aggregate windows
	}
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "microscope") {
		t.Fatalf("microscope status line missing:\n%s", plain)
	}
	// A far row is a time-weighted aggregate window, NOT a restored book.
	if !strings.Contains(plain, "aggregate, not a restored book") {
		t.Fatalf("far row must be labelled an aggregate window:\n%s", plain)
	}
	if !strings.Contains(plain, "▸") {
		t.Fatalf("the cursor row must be marked ▸:\n%s", plain)
	}
}

func TestMicroscopeNewestRowIsExactBin(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = press(m, "up") // enter at the newest (live) row
	if !strings.Contains(stripANSI(m.View()), "exact ~100ms bin") {
		t.Fatalf("the newest held row is an exact live bin:\n%s", stripANSI(m.View()))
	}
}

func TestMicroscopeFreezeHandsRowToAssistant(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = press(m, "up")    // enter the microscope
	m = press(m, "enter") // freeze the cursor row → assistant
	if m.screen != screenLLM {
		t.Fatalf("freeze should open the assistant, got screen %v", m.screen)
	}
	if m.assistCtx == nil {
		t.Fatalf("freeze must package a context")
	}
	if m.assistCtx.Origin != news.OriginBookFreeze {
		t.Fatalf("book freeze should tag OriginBookFreeze, got %v", m.assistCtx.Origin)
	}
	if m.assistCtx.Headline != nil {
		t.Fatalf("a book freeze must NOT fake a news marker: %+v", m.assistCtx.Headline)
	}
	if m.assistCtx.Note == "" {
		t.Fatalf("book freeze should carry an honest note")
	}
}

func TestBookFreezeAssistantRendersOriginNote(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = press(m, "up")    // enter the microscope at the newest (live) row
	m = press(m, "enter") // freeze → assistant
	plain := stripANSI(m.View())
	for _, want := range []string{"ASSISTANT", "book freeze", "exact ~100ms bin", "placeholder"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("book-freeze assistant view missing %q:\n%s", want, plain)
		}
	}
	// It must NOT render a headline severity block — no faked news marker.
	if strings.Contains(plain, "severity") {
		t.Fatalf("book freeze must not render a news headline block:\n%s", plain)
	}
}

func TestMicroscopeSwitchSymbolResetsCursor(t *testing.T) {
	m := watchModel(t, &conn.MockGateway{})
	m = press(m, "up") // enter the microscope on the primary symbol
	if m.rowCursor < 0 {
		t.Fatalf("up should enter the microscope")
	}
	m = press(m, "x")
	m = press(m, m.instrumentFor("rsx", 11).Code) // switch to WIF
	if m.rowCursor != -1 {
		t.Fatalf("a symbol switch must reset the microscope (it indexed the old book): %d", m.rowCursor)
	}
}
