package book

import (
	"math"
	"testing"

	"rsx-term/wire"
)

func TestPositionLongBuild(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Buy, 100, 5)
	p.ApplyFill(wire.Buy, 110, 5)
	if p.Net != 10 || p.Cost != 1050 {
		t.Fatalf("Net=%d Cost=%d, want 10, 1050", p.Net, p.Cost)
	}
	entry, ok := p.Entry()
	if !ok || entry != 105 {
		t.Fatalf("Entry() = %d, %v; want 105, true", entry, ok)
	}
}

func TestPositionPartialReduceKeepsEntry(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Buy, 100, 5)
	p.ApplyFill(wire.Buy, 110, 5)
	p.ApplyFill(wire.Sell, 999, 3) // px irrelevant to a reducing fill's Cost math
	// Cost = 1050 * (10-3) / 10 = 735 (integer division)
	if p.Net != 7 || p.Cost != 735 {
		t.Fatalf("Net=%d Cost=%d, want 7, 735", p.Net, p.Cost)
	}
	entry, ok := p.Entry()
	if !ok || entry != 105 {
		t.Fatalf("Entry() = %d, %v; want 105, true", entry, ok)
	}
}

func TestPositionFullClose(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Buy, 100, 5)
	p.ApplyFill(wire.Buy, 110, 5)
	p.ApplyFill(wire.Sell, 999, 3)
	p.ApplyFill(wire.Sell, 999, 7) // closes the remaining 7 exactly
	if p.Net != 0 || p.Cost != 0 {
		t.Fatalf("Net=%d Cost=%d, want 0, 0 exactly", p.Net, p.Cost)
	}
	if !p.Flat() {
		t.Fatalf("Flat() = false after full close")
	}
}

func TestPositionFlipLongToShort(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Buy, 100, 5)
	p.ApplyFill(wire.Buy, 110, 5) // Net=10, Cost=1050
	p.ApplyFill(wire.Sell, 120, 15)
	// r = min(15,10) = 10; Cost = 1050*(10-10)/10 = 0; Net = 10-15 = -5
	// flipped (abs(signed)=15 > r=10): Cost = 120 * -5 = -600
	if p.Net != -5 || p.Cost != -600 {
		t.Fatalf("Net=%d Cost=%d, want -5, -600", p.Net, p.Cost)
	}
	entry, ok := p.Entry()
	if !ok || entry != 120 {
		t.Fatalf("Entry() = %d, %v; want 120, true", entry, ok)
	}
}

func TestPositionShortBuildAndReduce(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Sell, 100, 5)
	p.ApplyFill(wire.Sell, 110, 5)
	if p.Net != -10 || p.Cost != -1050 {
		t.Fatalf("Net=%d Cost=%d, want -10, -1050", p.Net, p.Cost)
	}
	entry, ok := p.Entry()
	if !ok || entry != 105 {
		t.Fatalf("Entry() = %d, %v; want 105, true", entry, ok)
	}

	p.ApplyFill(wire.Buy, 999, 3) // partial buy-back
	// Cost = -1050 * (10-3) / 10 = -735 (integer division)
	if p.Net != -7 || p.Cost != -735 {
		t.Fatalf("Net=%d Cost=%d, want -7, -735", p.Net, p.Cost)
	}
	entry, ok = p.Entry()
	if !ok || entry != 105 {
		t.Fatalf("Entry() = %d, %v; want 105, true", entry, ok)
	}
}

func TestPositionUpnlLong(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Buy, 100, 5)
	p.ApplyFill(wire.Buy, 110, 5) // Net=10, Cost=1050, Entry=105

	if got, ok := p.Upnl(120); !ok || got != 150 {
		t.Fatalf("Upnl(120) = %d, %v; want 150, true", got, ok)
	}
	if got, ok := p.Upnl(90); !ok || got != -150 {
		t.Fatalf("Upnl(90) = %d, %v; want -150, true", got, ok)
	}
}

func TestPositionUpnlShort(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Sell, 100, 5)
	p.ApplyFill(wire.Sell, 110, 5) // Net=-10, Cost=-1050, Entry=105

	if got, ok := p.Upnl(90); !ok || got != 150 {
		t.Fatalf("Upnl(90) = %d, %v; want 150, true", got, ok)
	}
	if got, ok := p.Upnl(120); !ok || got != -150 {
		t.Fatalf("Upnl(120) = %d, %v; want -150, true", got, ok)
	}
}

func TestPositionApplyFillRejectsMulOverflow(t *testing.T) {
	var p Position
	// px*qty overflows i64: the fill must be rejected, not silently wrapped
	// into a plausible-but-false Net/Cost (POSITION-I64-OVERFLOW-WRAPS-PNL).
	if ok := p.ApplyFill(wire.Buy, math.MaxInt64, 2); ok {
		t.Fatalf("ApplyFill reported success on an overflowing notional")
	}
	if p.Net != 0 || p.Cost != 0 {
		t.Fatalf("Net=%d Cost=%d, want unchanged 0,0 after a rejected fill", p.Net, p.Cost)
	}
}

func TestPositionApplyFillRejectsAddOverflow(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Buy, 1, math.MaxInt64)
	if p.Net != math.MaxInt64 {
		t.Fatalf("Net=%d, want %d after first fill", p.Net, int64(math.MaxInt64))
	}
	// Net+signed overflows i64: reject, leave the existing position intact.
	if ok := p.ApplyFill(wire.Buy, 1, 1); ok {
		t.Fatalf("ApplyFill reported success on an overflowing Net add")
	}
	if p.Net != math.MaxInt64 {
		t.Fatalf("Net=%d, want unchanged %d after a rejected fill", p.Net, int64(math.MaxInt64))
	}
}

func TestPositionUpnlRejectsMulOverflow(t *testing.T) {
	var p Position
	p.ApplyFill(wire.Buy, 1, math.MaxInt64/2) // Net=MaxInt64/2, Cost=MaxInt64/2
	if _, ok := p.Upnl(4); ok {
		t.Fatalf("Upnl reported success on an overflowing Net*mark")
	}
}

func TestPositionZeroValueIsFlat(t *testing.T) {
	var p Position
	if !p.Flat() {
		t.Fatalf("Flat() = false on zero-value Position")
	}
	if _, ok := p.Entry(); ok {
		t.Fatalf("Entry() ok on zero-value Position")
	}
	if _, ok := p.Upnl(100); ok {
		t.Fatalf("Upnl() ok on zero-value Position")
	}
}
