package ui

import (
	"flag"
	"os"
	"path/filepath"
	"testing"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
)

// updateFlag rewrites the golden instead of asserting against it.
var updateFlag = flag.Bool("update", false, "rewrite golden files")

// updateGolden reports whether this run regenerates goldens.
func updateGolden() bool { return *updateFlag }

// TestDomViewGolden locks the classic DOM view byte-for-byte: with the stream
// flag off, the rendered frame (ANSI-stripped) must equal the checked-in
// golden. Any streaming-mode work that drifts the default view fails here.
// Regenerate deliberately with: go test ./ui -run TestDomViewGolden -update
func TestDomViewGolden(t *testing.T) {
	m := newModel(&conn.MockGateway{})
	m = apply(m, tea.WindowSizeMsg{Width: 120, Height: 32})
	for _, msg := range conn.DemoScript() {
		m = apply(m, msg)
	}
	got := stripANSI(m.View())

	path := filepath.Join("testdata", "dom_view.golden")
	if updateGolden() {
		if err := os.MkdirAll("testdata", 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(got), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read golden (run with -update to create): %v", err)
	}
	if got != string(want) {
		t.Fatalf("DOM view drifted from golden (flag off must be byte-for-byte unchanged).\n--- got ---\n%s\n--- want ---\n%s", got, want)
	}
}
