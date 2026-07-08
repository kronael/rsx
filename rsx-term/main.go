// Command rsx-term is the RSX single-symbol perps trading terminal. With
// RSX_GW_URL=mock it runs a fully offline scripted demo (no network); any
// other value selects the live gateway + marketdata path (increment 4).
// See specs/2/55-terminal.md.
package main

import (
	"errors"
	"fmt"
	"os"
	"strconv"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/ui"
)

// Symbol is the single market this terminal trades (playground PENGU-PERP).
// Multi-market is a future navigation layer, not a rewrite (specs/2/55).
const Symbol = "PENGU-PERP"

// demoTick paces the offline demo so the scripted feed visibly streams in.
const demoTick = 30 * time.Millisecond

func main() {
	// RSX_GW_URL: private (order) gateway URL. The literal "mock" runs the
	// offline demo with no network. Default is the dev gateway WS.
	gwURL := envOr("RSX_GW_URL", "ws://127.0.0.1:8080")
	// RSX_MD_URL: public marketdata URL.
	mdURL := envOr("RSX_MD_URL", "ws://127.0.0.1:8180")
	// RSX_TUI_USER: the trader's u32 user id.
	userID := envU32("RSX_TUI_USER", 0)
	// RSX_TUI_SYMBOL: the u32 symbol id (playground PENGU-PERP = 10).
	symbolID := envU32("RSX_TUI_SYMBOL", 10)
	// RSX_GW_JWT_SECRET: gateway JWT secret (dev default).
	jwtSecret := envOr("RSX_GW_JWT_SECRET", "rsx-dev-secret-not-for-prod-padpad")

	if gwURL == "mock" {
		runMock(symbolID)
		return
	}

	err := runLive(liveConfig{
		gwURL:     gwURL,
		mdURL:     mdURL,
		jwtSecret: jwtSecret,
		userID:    userID,
		symbolID:  symbolID,
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, "rsx-term:", err)
		os.Exit(1)
	}
}

// runMock builds the model over an offline MockGateway and streams the scripted
// demo feed into the running program.
func runMock(symbolID uint32) {
	mock := &conn.MockGateway{}
	model := ui.New(ui.Config{
		Symbol:     Symbol,
		SymbolID:   symbolID,
		Endpoint:   "mock://demo",
		MdEndpoint: "mock://demo",
		Sub:        mock,
	})
	p := tea.NewProgram(model, tea.WithAltScreen())
	go feedDemo(p)
	if _, err := p.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "rsx-term:", err)
		os.Exit(1)
	}
}

// feedDemo replays the scripted offline demo into the running program, pacing
// each message so the demo visibly streams in.
func feedDemo(p *tea.Program) {
	for _, msg := range conn.DemoScript() {
		p.Send(msg)
		time.Sleep(demoTick)
	}
}

// liveConfig is everything the live gateway + marketdata path needs.
type liveConfig struct {
	gwURL     string
	mdURL     string
	jwtSecret string
	userID    uint32
	symbolID  uint32
}

// runLive is the live gateway + marketdata path. Increment 4 replaces this
// body with real WS conns feeding the model; for now it is a stub so the
// binary fails fast instead of hanging on a missing gateway.
func runLive(cfg liveConfig) error {
	return errors.New("live gateway/marketdata conns land in increment 4 — run with RSX_GW_URL=mock")
}

// envOr returns the env var value, or def when unset or empty.
func envOr(key, def string) string {
	if v, ok := os.LookupEnv(key); ok && v != "" {
		return v
	}
	return def
}

// envU32 parses a u32 env var, or def when unset/empty. A present-but-malformed
// value (bad digits, or out of u32 range) is a hard error — a typo'd id must
// not silently trade as the wrong user/symbol.
func envU32(key string, def uint32) uint32 {
	v, ok := os.LookupEnv(key)
	if !ok || v == "" {
		return def
	}
	n, err := strconv.ParseUint(v, 10, 32)
	if err != nil {
		fmt.Fprintf(os.Stderr, "rsx-term: %s=%q is not a valid u32: %v\n", key, v, err)
		os.Exit(1)
	}
	return uint32(n)
}
