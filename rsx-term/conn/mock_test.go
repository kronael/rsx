package conn

import (
	"testing"

	"rsx-term/ui"
	"rsx-term/wire"
)

func TestMockRecords(t *testing.T) {
	m := &MockGateway{}
	if err := m.Submit(wire.OrderReq{Side: wire.Buy, Px: 100, Qty: 2, Tif: wire.Gtc}); err != nil {
		t.Fatalf("submit err: %v", err)
	}
	if err := m.Cancel("c1"); err != nil {
		t.Fatalf("cancel err: %v", err)
	}
	if len(m.Submitted) != 1 {
		t.Fatalf("submitted = %d, want 1", len(m.Submitted))
	}
	if len(m.Cancelled) != 1 || m.Cancelled[0] != "c1" {
		t.Fatalf("cancelled = %v", m.Cancelled)
	}
}

func TestMockDownErrorsAndRecordsNothing(t *testing.T) {
	m := &MockGateway{Down: true}
	if err := m.Submit(wire.OrderReq{Px: 1, Qty: 1}); err == nil {
		t.Fatalf("down Submit did not error")
	}
	if err := m.Cancel("c1"); err == nil {
		t.Fatalf("down Cancel did not error")
	}
	if len(m.Submitted) != 0 || len(m.Cancelled) != 0 {
		t.Fatalf("down mock recorded: %d submitted, %d cancelled", len(m.Submitted), len(m.Cancelled))
	}
}

// TestDemoScriptFolds folds the whole offline demo through a real ui.Model and
// asserts it ends in the derived LONG 14 @ 9998 position, never panicking.
// conn importing ui here is safe: ui does not import conn, so there is no cycle.
func TestDemoScriptFolds(t *testing.T) {
	cur := ui.New(ui.Config{
		Symbol:     "PENGU-PERP",
		SymbolID:   10,
		Endpoint:   "mock://demo",
		MdEndpoint: "mock://demo",
		Sub:        &MockGateway{},
	})
	for _, msg := range DemoScript() {
		next, _ := cur.Update(msg)
		cur = next.(ui.Model)
	}
	pos := cur.Position()
	if pos.Net != 14 {
		t.Fatalf("net = %d, want 14", pos.Net)
	}
	entry, ok := pos.Entry()
	if !ok || entry != 9998 {
		t.Fatalf("entry = %d ok = %v, want 9998 true", entry, ok)
	}
}
