package wire

import (
	"fmt"
	"strings"
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

func TestFolderTimeoutIsNotATerminal(t *testing.T) {
	f := NewFolder()
	ev, ok := f.Fold(`{"U":["`+oidHex(7)+`",3,0,0,10]}`, time.Unix(0, 0))
	if !ok {
		t.Fatal("expected event")
	}
	timedOut, ok := ev.(TimedOut)
	if !ok {
		t.Fatalf("expected TimedOut, got %T", ev)
	}
	if timedOut.Oid != 7 {
		t.Errorf("oid = %d want 7", timedOut.Oid)
	}
}

func TestFolderDuplicateCidPairsLostFirstAnswer(t *testing.T) {
	f := NewFolder()
	now := time.Unix(0, 0)
	f.Sent(OrderReq{Symbol: 12, Side: Sell, Px: 100, Qty: 7}, "cid-a", now)
	f.Sent(OrderReq{Symbol: 12, Side: Sell, Px: 100, Qty: 7}, "cid-a", now)
	f.Sent(OrderReq{Symbol: 3, Side: Buy, Px: 90, Qty: 2}, "cid-b", now)

	ev, ok := f.Fold(`{"E":[1011,"duplicate cid","`+oidHex(5)+`","cid-a"]}`, now)
	if !ok {
		t.Fatal("expected event")
	}
	rej := ev.(Rejected)
	if rej.Reason != "1011: duplicate cid" {
		t.Errorf("reason = %q", rej.Reason)
	}
	if len(f.pending) != 1 || f.pending[0].cid != "cid-b" {
		t.Fatalf("resend pending entries not dropped: %+v", f.pending)
	}
	ev, _ = f.Fold(`{"F":["`+oidHex(5)+`","`+oidHex(6)+`",100,7,0,0]}`, now)
	fill := ev.(Fill)
	if fill.Side != Sell || fill.Symbol != 12 {
		t.Fatalf("fill should carry the paired order: %+v", fill)
	}
}

func TestFolderDuplicateCidKeepsKnownPairing(t *testing.T) {
	f := NewFolder()
	now := time.Unix(0, 0)
	f.Sent(OrderReq{Side: Sell, Qty: 7}, "cid-a", now)
	f.Fold(`{"U":["`+oidHex(5)+`",1,0,7,0]}`, now)
	f.Sent(OrderReq{Side: Sell, Qty: 7}, "cid-a", now)

	f.Fold(`{"E":[1011,"duplicate cid","`+oidHex(5)+`","cid-a"]}`, now)
	if len(f.pending) != 0 {
		t.Fatalf("resend pending entry not dropped: %+v", f.pending)
	}
	if f.oidSide[5].side != Sell {
		t.Fatalf("known pairing lost: %+v", f.oidSide[5])
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

func TestFoldFillRecoversSymbol(t *testing.T) {
	f := NewFolder()
	f.Sent(OrderReq{Symbol: 12, Side: Sell, Px: 100, Qty: 7}, "cid-1", time.Unix(0, 0))
	// Accept pairs the pending order (qty 7) to the oid.
	if _, ok := f.Fold(`{"U":["00000000000000ff",1,0,7,0]}`, time.Unix(0, 1000)); !ok {
		t.Fatalf("accept did not fold")
	}
	ev, ok := f.Fold(`{"F":["00000000000000ff","00000000000000ee",100,7,0,0]}`, time.Unix(0, 2000))
	if !ok {
		t.Fatalf("fill did not fold")
	}
	fill := ev.(Fill)
	if fill.Symbol != 12 || fill.Side != Sell {
		t.Fatalf("fill should recover symbol+side from the paired order: %+v", fill)
	}
}

func TestEncodeNewCarriesRoutedSymbol(t *testing.T) {
	frame := EncodeNew(9, "cid-1", OrderReq{Symbol: 9, Side: Buy, Px: 5, Qty: 6})
	if !strings.HasPrefix(frame, `{"N":[9,`) {
		t.Fatalf("N frame should route to symbol 9: %s", frame)
	}
}
