package wire

import (
	"fmt"
	"testing"
	"time"
)

func TestEncodeNewGtcBuy(t *testing.T) {
	got := EncodeNew(10, "00000000000000000001", OrderReq{Side: Buy, Px: 50_000, Qty: 5, Tif: Gtc})
	want := `{"N":[10,0,50000,5,"00000000000000000001",0,0,0]}`
	if got != want {
		t.Fatalf("got %s want %s", got, want)
	}
}

func TestEncodeNewIocSellReduceOnlyPostOnly(t *testing.T) {
	got := EncodeNew(10, "00000000000000000002", OrderReq{Side: Sell, Px: 49_000, Qty: 3, Tif: Ioc, ReduceOnly: true, PostOnly: true})
	want := `{"N":[10,1,49000,3,"00000000000000000002",1,1,1]}`
	if got != want {
		t.Fatalf("got %s want %s", got, want)
	}
}

func TestEncodeCancel(t *testing.T) {
	got := EncodeCancel("00000000000000000042")
	want := `{"C":["00000000000000000042"]}`
	if got != want {
		t.Fatalf("got %s want %s", got, want)
	}
}

func TestCidZeroPadding(t *testing.T) {
	cases := []struct {
		n    uint64
		want string
	}{
		{0, "00000000000000000000"},
		{1, "00000000000000000001"},
		{123456789, "00000000000123456789"},
	}
	for _, c := range cases {
		if got := Cid(c.n); got != c.want {
			t.Errorf("Cid(%d) = %s want %s", c.n, got, c.want)
		}
	}
}

func TestIsHeartbeat(t *testing.T) {
	if !IsHeartbeat(`{"H":[12345]}`) {
		t.Error("expected H frame to be a heartbeat")
	}
	if IsHeartbeat(`{"U":[1,2,3]}`) {
		t.Error("U frame is not a heartbeat")
	}
	if IsHeartbeat(`not json`) {
		t.Error("malformed text is not a heartbeat")
	}
}

func TestOidTo64(t *testing.T) {
	// 32-char hex; low 16 chars decode to the expected value exactly.
	hex := "0000000000000000000000000000ff"
	if got := OidTo64(hex); got != 0xff {
		t.Errorf("OidTo64(%s) = %#x want 0xff", hex, got)
	}
	hex2 := fmt.Sprintf("%032x", uint64(0xdeadbeefcafef00d))
	if got := OidTo64(hex2); got != 0xdeadbeefcafef00d {
		t.Errorf("OidTo64(%s) = %#x want 0xdeadbeefcafef00d", hex2, got)
	}
	// Short string (< 16 hex chars): parses the whole thing.
	if got := OidTo64("abcd"); got != 0xabcd {
		t.Errorf("OidTo64(abcd) = %#x want 0xabcd", got)
	}
	if got := OidTo64("not-hex-not-hex-not-hex"); got != 0 {
		t.Errorf("OidTo64(garbage) = %#x want 0", got)
	}
}

func oidHex(n uint64) string {
	return fmt.Sprintf("%032x", n)
}

func TestFolderAcceptPairsSideAndMeasuresRtt(t *testing.T) {
	f := NewFolder()
	sent := time.Unix(0, 1000)
	f.Sent(OrderReq{Side: Buy, Px: 50_000, Qty: 5, Tif: Gtc}, "00000000000000000001", sent)

	now := time.Unix(0, 1500)
	ev, ok := f.Fold(`{"U":["`+oidHex(5)+`",1,0,5,0]}`, now)
	if !ok {
		t.Fatal("expected an event")
	}
	acc, ok := ev.(Accepted)
	if !ok {
		t.Fatalf("expected Accepted, got %T", ev)
	}
	if acc.Order.Side != Buy {
		t.Errorf("side = %v want Buy", acc.Order.Side)
	}
	if acc.RttNs != 500 {
		t.Errorf("rtt = %d want 500", acc.RttNs)
	}
}

func TestFolderOutOfOrderAcksKeepCorrectSide(t *testing.T) {
	f := NewFolder()
	now := time.Unix(0, 0)
	f.Sent(OrderReq{Side: Buy, Px: 50_000, Qty: 5, Tif: Gtc}, "cid-buy", now)
	f.Sent(OrderReq{Side: Sell, Px: 51_000, Qty: 3, Tif: Gtc}, "cid-sell", now)

	// qty=3 (Sell) acked first.
	ev, ok := f.Fold(`{"U":["`+oidHex(3)+`",1,0,3,0]}`, now)
	if !ok {
		t.Fatal("expected event")
	}
	acc := ev.(Accepted)
	if acc.Order.Side != Sell {
		t.Errorf("first ack side = %v want Sell", acc.Order.Side)
	}

	// qty=5 (Buy) acked second.
	ev, ok = f.Fold(`{"U":["`+oidHex(5)+`",1,0,5,0]}`, now)
	if !ok {
		t.Fatal("expected event")
	}
	acc = ev.(Accepted)
	if acc.Order.Side != Buy {
		t.Errorf("second ack side = %v want Buy", acc.Order.Side)
	}
}

func TestFolderRejectDoesNotConsumePending(t *testing.T) {
	f := NewFolder()
	now := time.Unix(0, 0)
	f.Sent(OrderReq{Side: Buy, Px: 50_000, Qty: 5, Tif: Gtc}, "cid-a", now)  // A, earlier
	f.Sent(OrderReq{Side: Sell, Px: 51_000, Qty: 7, Tif: Gtc}, "cid-b", now) // B, later, rejected fast

	// B rejected (status 3): no qty in play, must not consume a pending.
	ev, ok := f.Fold(`{"U":["`+oidHex(7)+`",3,0,0,42]}`, now)
	if !ok {
		t.Fatal("expected event")
	}
	if _, isRejected := ev.(Rejected); !isRejected {
		t.Fatalf("expected Rejected, got %T", ev)
	}

	// A still accepted correctly (Buy).
	ev, ok = f.Fold(`{"U":["`+oidHex(5)+`",1,0,5,0]}`, now)
	if !ok {
		t.Fatal("expected event")
	}
	acc := ev.(Accepted)
	if acc.Order.Side != Buy {
		t.Errorf("side = %v want Buy", acc.Order.Side)
	}
}

func TestFolderFillPairedSide(t *testing.T) {
	f := NewFolder()
	now := time.Unix(0, 0)
	f.Sent(OrderReq{Side: Sell, Px: 49_000, Qty: 7, Tif: Gtc}, "cid-sell", now)
	if _, ok := f.Fold(`{"U":["`+oidHex(7)+`",1,0,7,0]}`, now); !ok {
		t.Fatal("expected accept event")
	}

	// F names our oid as the maker (taker is unpaired) -> maker side used.
	ev, ok := f.Fold(`{"F":["`+oidHex(999)+`","`+oidHex(7)+`",49000,7,0,0]}`, now)
	if !ok {
		t.Fatal("expected fill event")
	}
	fill := ev.(Fill)
	if fill.Side != Sell {
		t.Errorf("fill side = %v want Sell", fill.Side)
	}
	if fill.Oid != 7 {
		t.Errorf("fill oid = %d want 7", fill.Oid)
	}
}

func TestFolderFillUnknownSidesFallBackToBuy(t *testing.T) {
	f := NewFolder()
	now := time.Unix(0, 0)
	ev, ok := f.Fold(`{"F":["`+oidHex(1)+`","`+oidHex(2)+`",50000,3,0,0]}`, now)
	if !ok {
		t.Fatal("expected fill event")
	}
	fill := ev.(Fill)
	if fill.Side != Buy {
		t.Errorf("fallback side = %v want Buy", fill.Side)
	}
	if fill.Oid != 1 {
		t.Errorf("fallback oid = %d want taker oid 1", fill.Oid)
	}
	if f.UnknownSideFills != 1 {
		t.Errorf("UnknownSideFills = %d want 1", f.UnknownSideFills)
	}
}

func TestFolderDoneStatuses(t *testing.T) {
	f := NewFolder()
	now := time.Unix(0, 0)
	f.Sent(OrderReq{Side: Buy, Px: 50_000, Qty: 5, Tif: Gtc}, "cid-a", now)
	ev, ok := f.Fold(`{"U":["`+oidHex(5)+`",0,5,0,0]}`, now)
	if !ok {
		t.Fatal("expected event")
	}
	if _, isDone := ev.(Done); !isDone {
		t.Fatalf("status 0: expected Done, got %T", ev)
	}

	f2 := NewFolder()
	f2.Sent(OrderReq{Side: Buy, Px: 50_000, Qty: 5, Tif: Gtc}, "cid-b", now)
	ev, ok = f2.Fold(`{"U":["`+oidHex(6)+`",2,0,5,0]}`, now)
	if !ok {
		t.Fatal("expected event")
	}
	if _, isDone := ev.(Done); !isDone {
		t.Fatalf("status 2: expected Done, got %T", ev)
	}
}

func TestFolderErrorFrame(t *testing.T) {
	f := NewFolder()
	ev, ok := f.Fold(`{"E":["BAD_INPUT","malformed order"]}`, time.Unix(0, 0))
	if !ok {
		t.Fatal("expected event")
	}
	rej, ok := ev.(Rejected)
	if !ok {
		t.Fatalf("expected Rejected, got %T", ev)
	}
	want := "BAD_INPUT: malformed order"
	if rej.Reason != want {
		t.Errorf("reason = %q want %q", rej.Reason, want)
	}
}

func TestFolderUnknownAndMalformedFrames(t *testing.T) {
	f := NewFolder()
	if _, ok := f.Fold(`{"Z":[1,2,3]}`, time.Unix(0, 0)); ok {
		t.Error("unknown key should yield no event")
	}
	if _, ok := f.Fold(`not json at all`, time.Unix(0, 0)); ok {
		t.Error("malformed JSON should yield no event")
	}
	if _, ok := f.Fold(`{"H":[123]}`, time.Unix(0, 0)); ok {
		t.Error("heartbeat should yield no event")
	}
}
