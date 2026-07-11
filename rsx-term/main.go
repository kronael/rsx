// Command rsx-term is the RSX single-symbol perps trading terminal. With
// RSX_GW_URL=mock it runs a fully offline scripted demo (no network); any
// other value selects the live gateway + marketdata path (increment 4).
// See specs/2/55-terminal.md.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/news"
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
	// RSX_TUI_THEME=colorblind swaps the bid/ask red-green pair for a
	// deuteranopia-safe blue/orange. Must run before any style is rendered.
	ui.UseTheme(os.Getenv("RSX_TUI_THEME"))
	// RSX_TERM_STREAM=1 selects the streaming "text Bookmap" heatmap view;
	// unset (the default) keeps the classic DOM three-column view.
	stream := os.Getenv("RSX_TERM_STREAM") == "1"
	// RSX_TERM_VENUE: rsx (default, no external calls) | hyperliquid
	// (standalone read-only HL terminal) | both (RSX primary + HL breadth).
	venueSel := envOr("RSX_TERM_VENUE", "rsx")

	if venueSel == "hyperliquid" {
		if err := runHL(); err != nil {
			fmt.Fprintln(os.Stderr, "rsx-term:", err)
			os.Exit(1)
		}
		return
	}
	var hlCfg *ui.VenueConfig
	var hl *conn.HL
	if venueSel == "both" || venueSel == "rsx,hyperliquid" {
		hlCfg, hl = hlVenue()
	}

	if gwURL == "mock" {
		priceDec, qtyDec, tick := displayConfig("", symbolID)
		runMock(symbolID, priceDec, qtyDec, tick, stream, hlCfg, hl)
		return
	}
	priceDec, qtyDec, tick := displayConfig(gwURL, symbolID)

	err := runLive(liveConfig{
		gwURL:       gwURL,
		mdURL:       mdURL,
		jwtSecret:   jwtSecret,
		userID:      userID,
		symbolID:    symbolID,
		priceDec:    priceDec,
		qtyDec:      qtyDec,
		tick:        tick,
		stream:      stream,
		instruments: watchInstruments(gwURL, symbolID, priceDec, qtyDec, tick, stream),
		hlCfg:       hlCfg,
		hl:          hl,
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, "rsx-term:", err)
		os.Exit(1)
	}
}

// hlCoins is the watched Hyperliquid coin set: RSX_TERM_HL_COINS (comma list,
// "all" = whole universe) or the curated default.
func hlCoins() []string {
	raw := os.Getenv("RSX_TERM_HL_COINS")
	if raw == "all" {
		return nil
	}
	if raw == "" {
		return conn.DefaultHLCoins
	}
	return strings.Split(raw, ",")
}

// hlVenue fetches the Hyperliquid universe and builds the extra read-only
// venue + its source. A fetch failure degrades to no HL venue (warned), never
// blocks the RSX terminal.
func hlVenue() (*ui.VenueConfig, *conn.HL) {
	meta, err := conn.FetchHLMeta()
	if err != nil {
		fmt.Fprintf(os.Stderr, "rsx-term: hyperliquid meta fetch failed (%v); continuing without it\n", err)
		return nil, nil
	}
	hl := conn.NewHL(meta, hlCoins())
	return &ui.VenueConfig{
		Name:        conn.HLVenueName,
		Code:        "h",
		Instruments: hlInstruments(hl),
		Sub:         nil, // read-only: HL trading needs EIP-712 signing (TODO, see conn/hyperliquid.go)
	}, hl
}

// hlInstruments maps the source's universe to UI instruments.
func hlInstruments(hl *conn.HL) []ui.Instrument {
	out := make([]ui.Instrument, 0, len(hl.Instruments()))
	for _, ins := range hl.Instruments() {
		out = append(out, ui.Instrument{
			ID:       ins.ID,
			Name:     ins.Coin,
			PriceDec: ins.PriceDec,
			QtyDec:   ins.QtyDec,
			Tick:     1,
			Sector:   conn.SectorOf(ins.Coin),
		})
	}
	return out
}

// keyOverrides loads the RSX_TERM_KEYMAP JSON file ({"action":"key"}) when
// set. A missing/broken file warns and keeps the defaults — startup never
// blocks on config sugar (the UI still flags a rejected map loudly).
func keyOverrides() map[string]string {
	path := os.Getenv("RSX_TERM_KEYMAP")
	if path == "" {
		return nil
	}
	data, err := os.ReadFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "rsx-term: keymap %s unreadable (%v); defaults active\n", path, err)
		return nil
	}
	var overrides map[string]string
	if err := json.Unmarshal(data, &overrides); err != nil {
		fmt.Fprintf(os.Stderr, "rsx-term: keymap %s is not a JSON object of action→key (%v); defaults active\n", path, err)
		return nil
	}
	return overrides
}

// newsSource builds the headline source: the live Tree of Alpha reader when
// RSX_TERM_NEWS=1 — the ONLY place the news feed dials, and an explicit
// opt-in, so default/offline/CI runs never touch the network. nil = news.Off.
func newsSource(ctx context.Context) news.Source {
	if os.Getenv("RSX_TERM_NEWS") != "1" {
		return nil
	}
	src := news.NewTreeOfAlpha()
	src.Start(ctx)
	return src
}

// runHL is the standalone read-only Hyperliquid terminal: the whole app
// (book/pair/news screens) over HL market data, no RSX cluster needed.
// Streaming is forced — the DOM view is an RSX order-entry screen.
func runHL() error {
	meta, err := conn.FetchHLMeta()
	if err != nil {
		return fmt.Errorf("hyperliquid meta: %w", err)
	}
	hl := conn.NewHL(meta, hlCoins())
	instruments := hlInstruments(hl)
	if len(instruments) == 0 {
		return fmt.Errorf("hyperliquid: no instruments after coin filter")
	}
	first := instruments[0]
	model := ui.New(ui.Config{
		Symbol:       first.Name,
		SymbolID:     first.ID,
		Endpoint:     "wss://api.hyperliquid.xyz/ws",
		MdEndpoint:   "wss://api.hyperliquid.xyz/ws",
		Venue:        conn.HLVenueName,
		Sub:          nil, // read-only (see conn/hyperliquid.go TODO)
		PriceDec:     first.PriceDec,
		QtyDec:       first.QtyDec,
		Tick:         first.Tick,
		Stream:       true,
		Instruments:  instruments,
		LotNotional:  envI64("RSX_TERM_LOT", 0),
		MaxNotional:  envI64("RSX_TERM_MAX_NOTIONAL", 0),
		News:         newsSource(context.Background()),
		KeyOverrides: keyOverrides(),
	})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	hl.Start(ctx)
	defer hl.Close()
	p := tea.NewProgram(model, tea.WithAltScreen(), tea.WithMouseCellMotion())
	go drainEvents(p, hl.Events())
	_, err = p.Run()
	return err
}

// watchInstruments builds the streaming watchlist: every symbol the gateway
// serves (decimals/tick from /v1/symbols) named/coded via RSX_TERM_WATCH
// ("id:name[:code[:multPct]]", comma-separated — the endpoint carries no
// names yet, so unnamed ids show as SYM-<id>). The primary symbol always
// leads. DOM mode (stream off) keeps the single-symbol list.
func watchInstruments(gwURL string, primary uint32, priceDec, qtyDec int, tick int64, stream bool) []ui.Instrument {
	primaryIns := ui.Instrument{
		ID: primary, Name: Symbol, PriceDec: priceDec, QtyDec: qtyDec, Tick: tick,
	}
	if !stream {
		return []ui.Instrument{primaryIns}
	}
	names, codes, mults := parseWatchEnv(os.Getenv("RSX_TERM_WATCH"))
	if n, ok := names[primary]; ok {
		primaryIns.Name = n
	}
	primaryIns.Code = codes[primary]
	primaryIns.LotMult = mults[primary]
	out := []ui.Instrument{primaryIns}

	symbols, err := conn.FetchSymbols(gwURL)
	if err != nil {
		fmt.Fprintf(os.Stderr, "rsx-term: symbol list fetch failed (%v); single-symbol watchlist\n", err)
		return out
	}
	for _, s := range symbols {
		if s.ID == primary {
			continue
		}
		name, ok := names[s.ID]
		if !ok {
			name = fmt.Sprintf("SYM-%d", s.ID)
		}
		out = append(out, ui.Instrument{
			ID:       s.ID,
			Name:     name,
			Code:     codes[s.ID],
			PriceDec: s.PriceDec,
			QtyDec:   s.QtyDec,
			Tick:     s.TickSize,
			LotMult:  mults[s.ID],
		})
	}
	return out
}

// parseWatchEnv parses RSX_TERM_WATCH: "id:name[:code[:multPct]]" entries,
// comma-separated. Malformed entries are skipped (config sugar must never
// stop the terminal).
func parseWatchEnv(raw string) (names map[uint32]string, codes map[uint32]string, mults map[uint32]int64) {
	names, codes, mults = map[uint32]string{}, map[uint32]string{}, map[uint32]int64{}
	for _, entry := range strings.Split(raw, ",") {
		parts := strings.Split(strings.TrimSpace(entry), ":")
		if len(parts) < 2 || parts[0] == "" {
			continue
		}
		id64, err := strconv.ParseUint(parts[0], 10, 32)
		if err != nil {
			continue
		}
		id := uint32(id64)
		names[id] = parts[1]
		if len(parts) > 2 && parts[2] != "" {
			codes[id] = parts[2]
		}
		if len(parts) > 3 {
			if mult, err := strconv.ParseInt(parts[3], 10, 64); err == nil && mult > 0 {
				mults[id] = mult
			}
		}
	}
	return names, codes, mults
}

// displayConfig resolves the symbol's display precision + tick. Base values
// come from the gateway's /v1/symbols (the authoritative per-symbol config) on
// the live path; gwURL == "" (mock) skips the fetch. A fetch failure falls back
// to PENGU's values (price 6, qty 4, tick 1) with a note — config discovery
// must never keep the terminal from starting. RSX_TUI_PRICE_DECIMALS /
// RSX_TUI_QTY_DECIMALS / RSX_TUI_TICK, when set, override whatever base won.
func displayConfig(gwURL string, symbolID uint32) (priceDec, qtyDec int, tick int64) {
	priceDec, qtyDec, tick = 6, 4, 1
	if gwURL != "" {
		if cfg, err := conn.FetchSymbolConfig(gwURL, symbolID); err == nil {
			priceDec, qtyDec, tick = cfg.PriceDec, cfg.QtyDec, cfg.TickSize
		} else {
			fmt.Fprintf(os.Stderr, "rsx-term: symbol config fetch failed (%v); using defaults\n", err)
		}
	}
	priceDec = int(envU32("RSX_TUI_PRICE_DECIMALS", uint32(priceDec)))
	qtyDec = int(envU32("RSX_TUI_QTY_DECIMALS", uint32(qtyDec)))
	tick = int64(envU32("RSX_TUI_TICK", uint32(tick)))
	return priceDec, qtyDec, tick
}

// runMock builds the model over an offline MockGateway and streams the scripted
// demo feed into the running program. Streaming mode watches the demo peer
// symbol too, so the pair view has breadth offline; an optional HL venue
// (RSX_TERM_VENUE=both) rides along exactly as on the live path.
func runMock(symbolID uint32, priceDec, qtyDec int, tick int64, stream bool, hlCfg *ui.VenueConfig, hl *conn.HL) {
	mock := &conn.MockGateway{}
	instruments := []ui.Instrument{
		{ID: symbolID, Name: Symbol, PriceDec: priceDec, QtyDec: qtyDec, Tick: tick},
	}
	if stream {
		// SOL-PERP per the exchange config: price_dec 4, qty_dec 6, tick 1.
		instruments = append(instruments, ui.Instrument{
			ID: conn.DemoPeerID, Name: "SOL-PERP", PriceDec: 4, QtyDec: 6, Tick: 1,
		})
	}
	model := ui.New(ui.Config{
		Symbol:       Symbol,
		SymbolID:     symbolID,
		Endpoint:     "mock://demo",
		MdEndpoint:   "mock://demo",
		Sub:          mock,
		PriceDec:     priceDec,
		QtyDec:       qtyDec,
		Tick:         tick,
		Stream:       stream,
		Instruments:  instruments,
		Venues:       extraVenues(hlCfg),
		LotNotional:  envI64("RSX_TERM_LOT", 0),
		MaxNotional:  envI64("RSX_TERM_MAX_NOTIONAL", 0),
		News:         newsSource(context.Background()),
		KeyOverrides: keyOverrides(),
	})
	p := tea.NewProgram(model, tea.WithAltScreen(), tea.WithMouseCellMotion())
	go feedDemo(p)
	startHL(p, hl)
	if _, err := p.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "rsx-term:", err)
		os.Exit(1)
	}
}

// extraVenues lifts an optional venue into the Config slice.
func extraVenues(hlCfg *ui.VenueConfig) []ui.VenueConfig {
	if hlCfg == nil {
		return nil
	}
	return []ui.VenueConfig{*hlCfg}
}

// startHL launches the HL source and its event pump (no-op without one).
// The source owns reconnects; Close rides on process exit.
func startHL(p *tea.Program, hl *conn.HL) {
	if hl == nil {
		return
	}
	hl.Start(context.Background())
	go drainEvents(p, hl.Events())
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
	gwURL       string
	mdURL       string
	jwtSecret   string
	userID      uint32
	symbolID    uint32
	priceDec    int
	qtyDec      int
	tick        int64
	stream      bool
	instruments []ui.Instrument
	hlCfg       *ui.VenueConfig
	hl          *conn.HL
}

// runLive builds a LiveGateway from cfg, connects both sockets, and runs
// the model over it. The connection's own goroutines (one per socket, see
// conn.LiveGateway) decode/fold wire frames into messages on Events();
// drainEvents is the only extra goroutine main.go adds, mirroring feedDemo.
func runLive(cfg liveConfig) error {
	live := conn.NewLiveGateway(cfg.gwURL, cfg.mdURL, cfg.jwtSecret, cfg.userID, cfg.symbolID)
	var watch []uint32
	for _, ins := range cfg.instruments {
		watch = append(watch, ins.ID)
	}
	live.WatchSymbols(watch)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	if err := live.Connect(ctx); err != nil {
		return err
	}
	defer live.Close()

	model := ui.New(ui.Config{
		Symbol:       Symbol,
		SymbolID:     cfg.symbolID,
		Endpoint:     cfg.gwURL,
		MdEndpoint:   cfg.mdURL,
		Sub:          live,
		PriceDec:     cfg.priceDec,
		QtyDec:       cfg.qtyDec,
		Tick:         cfg.tick,
		Stream:       cfg.stream,
		Instruments:  cfg.instruments,
		Venues:       extraVenues(cfg.hlCfg),
		LotNotional:  envI64("RSX_TERM_LOT", 0),
		MaxNotional:  envI64("RSX_TERM_MAX_NOTIONAL", 0),
		News:         newsSource(ctx),
		KeyOverrides: keyOverrides(),
	})
	p := tea.NewProgram(model, tea.WithAltScreen(), tea.WithMouseCellMotion())
	go drainEvents(p, live.Events())
	startHL(p, cfg.hl)
	_, err := p.Run()
	return err
}

// drainEvents forwards every message the live connection decodes into the
// running program, mirroring feedDemo's role for the offline path.
func drainEvents(p *tea.Program, events <-chan any) {
	for msg := range events {
		p.Send(msg)
	}
}

// envOr returns the env var value, or def when unset or empty.
func envOr(key, def string) string {
	if v, ok := os.LookupEnv(key); ok && v != "" {
		return v
	}
	return def
}

// envI64 parses an int64 env var, or def when unset/empty/malformed (sizing
// sugar — a bad value falls back rather than blocking launch).
func envI64(key string, def int64) int64 {
	v, ok := os.LookupEnv(key)
	if !ok || v == "" {
		return def
	}
	n, err := strconv.ParseInt(v, 10, 64)
	if err != nil {
		fmt.Fprintf(os.Stderr, "rsx-term: %s=%q is not a valid integer; using %d\n", key, v, def)
		return def
	}
	return n
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
