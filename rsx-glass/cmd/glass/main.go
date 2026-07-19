// Command glass is the desktop viewer: it streams a symbol's live order book and
// draws the rsx-term log-time heatmap on the GPU. This is the composition root —
// the ONE place the go-gl backend (package gpu) is wired in. It reuses the same
// live path the headless integration suite exercises (live.Client -> glass.Feed
// -> Grid), then hands the newest Grid to glass.Run against the GPU renderer.
//
// Config is env-only (RSX exchange convention):
//
//	RSX_MD_URL     market-data WebSocket URL; empty -> an in-process mock feed
//	               so the viewer runs standalone with no cluster.
//	RSX_SYMBOL_ID  the symbol to subscribe (default mockx.DefaultSymbolID).
//	RSX_SYMBOL     display name in the header/title (default "BTC-PERP").
//	RSX_TUI_THEME  "colorblind" swaps the bid/ask pair (rsx-term's theme seam).
//
// It must compile headless; it opens a window, so it runs on a desktop.
package main

import (
	"context"
	"errors"
	"log"
	"os"
	"os/signal"
	"runtime"
	"strconv"
	"syscall"
	"time"

	"rsx-glass"
	"rsx-glass/gpu"
	"rsx-glass/live"
	"rsx-glass/mockx"
)

// The heatmap grid is fixed at the size the model and goldens are built for
// (header 1 + body + ruler 1 + footer 2). renderTick paces glass.Run; vsync in
// the backend does the real pacing, so this is just an upper bound on wakeups.
const (
	gridW      = 72
	gridH      = 24
	renderTick = 16 * time.Millisecond
	binTick    = 100 * time.Millisecond // seal a heatmap row every 100ms (rsx-term cadence)
)

// config is the resolved run configuration.
type config struct {
	url      string
	symbol   string
	symbolID uint32
}

func main() {
	runtime.LockOSThread() // composition root owns the main thread: GLFW lifecycle+events and the GL context all live here
	log.SetFlags(0)
	glass.UseTheme(os.Getenv("RSX_TUI_THEME"))
	cfg := loadConfig()

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	renderer, err := gpu.New(gridW, gridH, "rsx-glass — "+cfg.symbol)
	if err != nil {
		log.Fatalf("open window (needs a display + OpenGL 3.3): %v", err)
	}
	defer renderer.Close() //nolint:errcheck // process is exiting; nothing to recover

	events, closer, err := dial(ctx, cfg)
	if err != nil {
		log.Fatalf("connect market data: %v", err)
	}
	defer closer()

	grids := make(chan *glass.Grid, 1)
	go pump(ctx, events, cfg, grids)

	if err := glass.Run(grids, renderer, renderTick); err != nil && !errors.Is(err, gpu.ErrClosed) {
		log.Printf("render loop stopped: %v", err)
	}
	stop() // window closed -> cancel ctx so the pump unwinds
}

// loadConfig reads the env-only configuration.
func loadConfig() config {
	symbolID := mockx.DefaultSymbolID
	if v := os.Getenv("RSX_SYMBOL_ID"); v != "" {
		if n, err := strconv.ParseUint(v, 10, 32); err == nil {
			symbolID = uint32(n)
		}
	}
	symbol := os.Getenv("RSX_SYMBOL")
	if symbol == "" {
		symbol = "BTC-PERP"
	}
	return config{url: os.Getenv("RSX_MD_URL"), symbol: symbol, symbolID: symbolID}
}

// dial sets up the market-data event stream: the real feed at cfg.url, or an
// in-process mock exchange when no URL is given. It returns the decoded events
// and a closer that tears the connection (and mock, if any) down.
func dial(ctx context.Context, cfg config) (<-chan any, func(), error) {
	url := cfg.url
	var server *mockx.Server
	if url == "" {
		s, err := mockx.New(mockx.Options{SymbolID: cfg.symbolID, KeepAliveHeartbeat: 500 * time.Millisecond})
		if err != nil {
			return nil, nil, err
		}
		server, url = s, s.URL
		log.Printf("no RSX_MD_URL: streaming the in-process mock feed at %s", url)
	}
	client := live.New(url, cfg.symbolID)
	client.Start(ctx)
	closer := func() {
		client.Close()
		if server != nil {
			server.Close()
		}
	}
	return client.Events(), closer, nil
}

// pump folds decoded market-data events into the reused model and publishes a
// fresh Grid every bin. It is the sole sender on grids and closes it on exit, so
// glass.Run returns cleanly when the context is cancelled. A named goroutine (not
// an inline closure) so its lifetime and back-pressure are legible.
func pump(ctx context.Context, events <-chan any, cfg config, grids chan *glass.Grid) {
	defer close(grids)
	feed := glass.NewFeed(cfg.symbol, cfg.symbolID, 1, gridW, gridH)
	bin := time.NewTicker(binTick)
	defer bin.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case ev, ok := <-events:
			if !ok {
				return
			}
			feed.Apply(ev, time.Now().UnixNano())
		case <-bin.C:
			now := time.Now().UnixNano()
			feed.Bin(now)
			g := glass.NewGrid(gridW, gridH)
			glass.Compose(g, feed.Scene())
			publish(grids, g)
		}
	}
}

// publish sends the newest grid without blocking the fold loop AND without
// dropping it: if the render side has not drained the previous grid, the STALE
// one is discarded so the newest takes the one slot. This is the Mailbox rule —
// newest-wins — not "keep whatever arrived first". With a single producer the
// drain-then-retry always terminates: after a drain the slot is free.
func publish(grids chan *glass.Grid, g *glass.Grid) {
	for {
		select {
		case grids <- g:
			return
		default:
		}
		select {
		case <-grids: // drop the stale pending grid, then retry with the newest
		default:
		}
	}
}
