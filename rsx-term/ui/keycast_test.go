package ui

import (
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
)

// keycastModel is streamModel with the keycast overlay on.
func keycastModel(t *testing.T) Model {
	t.Helper()
	m := streamModel(t, &conn.MockGateway{})
	m.cfg.Keycast = true
	return m
}

func TestKeycastChipShowsKeyAndKeymapLabel(t *testing.T) {
	m := keycastModel(t)
	m = press(m, "f")
	got := stripANSI(m.View())
	// The label is the dispatch table's help for actPlace — reused, not a
	// second string.
	if !strings.Contains(got, "⌨ f") {
		t.Fatalf("chip missing pressed key:\n%s", got)
	}
	if !strings.Contains(got, "place resting limit at the cursor") {
		t.Fatalf("chip missing keymap label:\n%s", got)
	}
}

func TestKeycastArrowGlyphAndParentheticalStripped(t *testing.T) {
	m := keycastModel(t)
	m = apply(m, tea.KeyMsg{Type: tea.KeyUp})
	got := stripANSI(m.View())
	if !strings.Contains(got, "⌨ ↑") {
		t.Fatalf("arrow key not rendered as glyph:\n%s", got)
	}
	// help is "microscope: cursor to an older row (freeze, not replay)" —
	// the chip drops the parenthetical.
	if !strings.Contains(got, "microscope: cursor to an older row") {
		t.Fatalf("chip missing row-up label:\n%s", got)
	}
	if strings.Contains(got, "freeze, not replay") {
		t.Fatalf("chip kept the parenthetical detail:\n%s", got)
	}
}

func TestKeycastClassKeysLabelled(t *testing.T) {
	m := keycastModel(t)
	m = press(m, "3")
	if got := stripANSI(m.View()); !strings.Contains(got, "arm a size preset") {
		t.Fatalf("digit preset not labelled from the class doc:\n%s", got)
	}
	m = press(m, "!")
	if got := stripANSI(m.View()); !strings.Contains(got, "cross NOW") {
		t.Fatalf("shifted digit not labelled from the class doc:\n%s", got)
	}
}

func TestKeycastCaptureModeShowsBareKey(t *testing.T) {
	// In the symbol switcher, letters are raw input — the chip must not
	// claim a book verb for them.
	m := keycastModel(t)
	m = press(m, "x") // enter the switcher
	m = press(m, "s") // typed symbol code, NOT "side: sell"
	got := stripANSI(m.View())
	if !strings.Contains(got, "⌨ s") {
		t.Fatalf("capture-mode key missing:\n%s", got)
	}
	if strings.Contains(got, "side: sell") {
		t.Fatalf("capture-mode key labelled with a book verb:\n%s", got)
	}
}

func TestKeycastOffRenderUnchanged(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	before := m.View()
	m = press(m, "b")
	after := m.View()
	if strings.Contains(after, "⌨") {
		t.Fatalf("keycast chip rendered with the flag off")
	}
	if before != after {
		// b flips the side; nothing on the book screen renders the side
		// without an order, so the frame must be byte-identical.
		t.Fatalf("flag-off render drifted after a keypress")
	}
}

func TestKeycastFadesOutOnBinTicks(t *testing.T) {
	m := keycastModel(t)
	m = press(m, "k")
	for i := 0; i < keycastTTL; i++ {
		m = apply(m, binTickMsg(time.Now()))
	}
	if got := stripANSI(m.View()); strings.Contains(got, "⌨") {
		t.Fatalf("chip did not age out after %d ticks:\n%s", keycastTTL, got)
	}
}
