package live_test

import (
	"bytes"
	"context"
	"image"
	"image/png"
	"os"
	"testing"
	"time"

	"rsx-glass"
	"rsx-glass/live"
	"rsx-glass/mockx"
	"rsx-glass/soft"

	"rsx-term/feed"
	"rsx-term/wire"
)

// The integration suite drives rsx-glass's WHOLE live path against the mock
// exchange over a real WebSocket: exchange wire (binary MdFrame) -> live.Client
// (rsx-term's wire.DecodeMd) -> glass.Feed (rsx-term's book model) -> Grid ->
// soft (pixels). Nothing here is stubbed but the exchange's data source; the
// decoder, the model, the Grid builder, and the renderer are the real ones.

const gridW, gridH = 72, 24

// seen counts the decoded event kinds the connector delivered, so a test can
// wait for full coverage of the feed's flows.
type seen struct {
	mdUp, mdDown    int
	snapshot, delta int
	bbo, trade      int
}

func (s seen) full() bool {
	// The scenario emits 1 snapshot, 4 deltas, 2 bbos, 4 trades.
	return s.snapshot >= 1 && s.delta >= 4 && s.bbo >= 2 && s.trade >= 4
}

// drive drains the client's events into a fresh Feed, sealing a bin per event
// so the heatmap fills, and returns once want is satisfied (or the deadline
// fires). The bin clock is the event count, not wall time, so the folded model
// is independent of socket timing.
func drive(t *testing.T, client *live.Client, symbolID uint32, want func(seen) bool) (*glass.Feed, seen) {
	t.Helper()
	f := glass.NewFeed("BTC-PERP", symbolID, 1, gridW, gridH)
	var got seen
	deadline := time.After(5 * time.Second)
	base := int64(1_000_000_000)
	step := 0
	for {
		if want(got) {
			return f, got
		}
		select {
		case ev, ok := <-client.Events():
			if !ok {
				return f, got
			}
			step++
			nowNs := base + int64(step)*int64(100*time.Millisecond)
			f.Apply(ev, nowNs)
			f.Bin(nowNs)
			record(&got, ev)
		case <-deadline:
			t.Fatalf("timed out; saw %+v", got)
			return f, got
		}
	}
}

func record(s *seen, ev any) {
	switch ev.(type) {
	case feed.MdUp:
		s.mdUp++
	case feed.MdDown:
		s.mdDown++
	case wire.Snapshot:
		s.snapshot++
	case wire.Delta:
		s.delta++
	case wire.Bbo:
		s.bbo++
	case wire.MdTrade:
		s.trade++
	}
}

// TestLiveHeatmapEndToEnd is the headline integration test: subscribe, snapshot,
// incremental deltas, BBO, trades, and heartbeats all flow from the mock through
// the real decoder and model into a rendered heatmap.
func TestLiveHeatmapEndToEnd(t *testing.T) {
	server, err := mockx.New(mockx.Options{KeepAliveHeartbeat: 15 * time.Millisecond})
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	client := live.New(server.URL, mockx.DefaultSymbolID)
	client.Start(ctx)
	defer client.Close()

	f, got := drive(t, client, mockx.DefaultSymbolID, seen.full)

	// Every feed flow was exercised.
	if got.mdUp < 1 {
		t.Errorf("no MdUp link event: %+v", got)
	}
	if !got.full() {
		t.Fatalf("did not observe every frame kind: %+v", got)
	}

	// The subscribe frame reached the server in the real shape.
	subs := server.Subscriptions()
	if len(subs) != 1 || subs[0].SymbolID != mockx.DefaultSymbolID || subs[0].Channels != live.DefaultChannels {
		t.Fatalf("subscribe not recorded as {sym:%d ch:%d}: %+v", mockx.DefaultSymbolID, live.DefaultChannels, subs)
	}

	// The model folded to an anchored, renderable heatmap.
	sc := f.Scene()
	if sc.Heat.MidPx() <= 0 {
		t.Fatal("heatmap never anchored from the live snapshot")
	}
	img := render(t, sc)
	assertRealHeatmap(t, img)

	// The client answered the server's heartbeats (liveness echo round-trips).
	waitFor(t, 2*time.Second, func() bool { return server.HeartbeatEchoes() > 0 })
	if server.HeartbeatEchoes() == 0 {
		t.Fatal("client never echoed a heartbeat")
	}

	// Optional visual dump for eyeballing the live-path render.
	if path := os.Getenv("RSX_GLASS_DUMP"); path != "" {
		writePNG(t, path, img)
	}
}

// TestLiveMalformedFrameSkipped drives the untrusted-feed rule end to end: the
// server slips a truncated binary frame in after the snapshot, and the client
// must skip it and keep folding every later frame.
func TestLiveMalformedFrameSkipped(t *testing.T) {
	server, err := mockx.New(mockx.Options{InjectMalformed: true})
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	client := live.New(server.URL, mockx.DefaultSymbolID)
	client.Start(ctx)
	defer client.Close()

	_, got := drive(t, client, mockx.DefaultSymbolID, seen.full)
	if !got.full() {
		t.Fatalf("a malformed frame broke the stream: %+v", got)
	}
}

// TestLiveReconnect drives the reconnect path: the server drops the first
// connection after the scenario, and the client must report MdDown, redial,
// re-subscribe, and receive the scenario again.
func TestLiveReconnect(t *testing.T) {
	server, err := mockx.New(mockx.Options{DropOnce: true})
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	client := live.New(server.URL, mockx.DefaultSymbolID)
	client.Start(ctx)
	defer client.Close()

	// Wait until we have connected twice (i.e. reconnected once) and folded a
	// full scenario on the second link.
	twiceUp := func(s seen) bool { return s.mdUp >= 2 && s.snapshot >= 2 }
	_, got := drive(t, client, mockx.DefaultSymbolID, twiceUp)

	if got.mdUp < 2 {
		t.Fatalf("expected a reconnect (>=2 MdUp), saw %+v", got)
	}
	if got.mdDown < 1 {
		t.Fatalf("expected an MdDown on the drop, saw %+v", got)
	}
	waitFor(t, 2*time.Second, func() bool { return len(server.Subscriptions()) >= 2 })
	if n := len(server.Subscriptions()); n < 2 {
		t.Fatalf("reconnect did not re-subscribe: %d subscribes", n)
	}
}

// render composes the scene into a Grid and rasterises it with the real soft
// renderer.
func render(t *testing.T, sc glass.Scene) *image.RGBA {
	t.Helper()
	g := glass.NewGrid(gridW, gridH)
	glass.Compose(g, sc)
	r, err := soft.New()
	if err != nil {
		t.Fatalf("soft.New: %v", err)
	}
	if err := r.Present(g); err != nil {
		t.Fatalf("Present: %v", err)
	}
	return r.Image()
}

// assertRealHeatmap checks the rendered frame is an actual heatmap, not a blank
// or uniform fill: it has the expected pixel dimensions, many distinct colours
// (the size ramp + glyphs), and real live-hue liquidity cells.
func assertRealHeatmap(t *testing.T, img *image.RGBA) {
	t.Helper()
	if img == nil {
		t.Fatal("nil framebuffer")
	}
	if img.Bounds().Dx() < gridW || img.Bounds().Dy() < gridH {
		t.Fatalf("framebuffer too small: %v", img.Bounds())
	}
	colors := map[uint32]int{}
	page := packRGBA(glass.Page)
	nonPage, liveish := 0, 0
	for i := 0; i+3 < len(img.Pix); i += 4 {
		c := uint32(img.Pix[i])<<24 | uint32(img.Pix[i+1])<<16 | uint32(img.Pix[i+2])<<8 | uint32(img.Pix[i+3])
		colors[c]++
		if c != page {
			nonPage++
		}
		// A pixel biased toward the live hue: strong green, modest red/blue —
		// the size ramp blends Page->Live, so folded liquidity paints these.
		if img.Pix[i+1] > img.Pix[i] && img.Pix[i+1] > img.Pix[i+2] && img.Pix[i+1] > 0x40 {
			liveish++
		}
	}
	if len(colors) < 8 {
		t.Fatalf("render is nearly uniform (%d distinct colours) — not a heatmap", len(colors))
	}
	if nonPage == 0 {
		t.Fatal("every pixel is the page background — the body rendered empty")
	}
	if liveish == 0 {
		t.Fatal("no live-hue liquidity cells — the size ramp did not paint the book")
	}
}

func packRGBA(c interface{ RGBA() (r, g, b, a uint32) }) uint32 {
	r, g, b, a := c.RGBA()
	return uint32(r>>8)<<24 | uint32(g>>8)<<16 | uint32(b>>8)<<8 | uint32(a>>8)
}

func waitFor(t *testing.T, d time.Duration, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(d)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
}

func writePNG(t *testing.T, path string, img *image.RGBA) {
	t.Helper()
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
