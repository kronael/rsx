package ui

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"rsx-term/conn"
	"rsx-term/wire"
)

// goldenCompare asserts an ANSI-stripped frame against a checked-in golden
// (regenerate deliberately with -update).
func goldenCompare(t *testing.T, name, got string) {
	t.Helper()
	path := filepath.Join("testdata", name)
	if updateGolden() {
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
		t.Fatalf("%s drifted from golden.\n--- got ---\n%s\n--- want ---\n%s", name, got, want)
	}
}

// TestBookViewGolden locks the depth screen's layout: live book with a
// building ask wall, a trade burst, an own resting order, and the cursor on
// the ruler.
func TestBookViewGolden(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	// A wall builds one tick above the ask touch; prints lift the offer.
	for i := 0; i < 6; i++ {
		m = apply(m, wire.Delta{Side: 1, Px: 10002, Qty: int64(200000 + 40000*i), Count: uint32(4 + i), Seq: uint64(10 + i)})
		m = apply(m, wire.MdTrade{Px: 10001, Qty: int64(9000 * (i + 1)), TakerSide: 0, Seq: uint64(30 + i)})
		m = apply(m, binTickMsg(time.Unix(1700000000+int64(i), 0)))
	}
	m = apply(m, wire.Accepted{Oid: 9, Cid: "c1", Order: wire.OrderReq{Side: wire.Buy, Px: 9998, Qty: 30000}})
	m = press(m, "j") // cursor to the best bid
	goldenCompare(t, "book_view.golden", stripANSI(m.View()))
}

// TestNewsViewFixedShape asserts the news screen's structural rows (the feed
// carries wall-clock strings, so it is shape-checked, not byte-locked).
func TestNewsViewFixedShape(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	plain := stripANSI(m.View())
	lines := strings.Split(plain, "\n")
	if len(lines) != 32 {
		t.Fatalf("news view = %d lines, want 32", len(lines))
	}
	if !strings.Contains(lines[1], "NEWS") {
		t.Fatalf("mode line should tag the NEWS screen: %q", lines[1])
	}
}
