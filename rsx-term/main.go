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

	"rsx-term/assistant"
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
	// offline demo with no network. Default is the dev gateway WS (:8088 —
	// the playground runs the RSX gateway there since arizuko's routd claims
	// :8080 in the combined demo).
	gwURL := envOr("RSX_GW_URL", "ws://127.0.0.1:8088")
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
		priceDec, qtyDec, tick := displayConfig(nil, symbolID)
		runMock(symbolID, priceDec, qtyDec, tick, stream, hlCfg, hl)
		return
	}
	symbols := fetchSymbolList(gwURL)
	priceDec, qtyDec, tick := displayConfig(symbols, symbolID)

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
		instruments: watchInstruments(symbols, symbolID, priceDec, qtyDec, tick, stream),
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

// assistSource builds the LLM chat client from RSX_TERM_ASSIST — the full
// arizuko /chat/{token} URL. Unset (the default) returns nil: the assistant
// pane stays the offline placeholder and makes zero dials, mirroring
// newsSource. The one env var is the whole gate; timeouts and topic are
// internal.
func assistSource() *assistant.Client {
	url := os.Getenv("RSX_TERM_ASSIST")
	if url == "" {
		return nil
	}
	return assistant.New(url)
}

// startAssist pumps the assistant client's event stream into the program (the
// reply frames fold in Update). No-op without a client, mirroring startHL.
func startAssist(p *tea.Program, assist *assistant.Client) {
	if assist == nil {
		return
	}
	go drainEvents(p, assist.Events())
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
	assist := assistSource()
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
		MaxNotional:  envI64("RSX_TERM_MAX_NOTIONAL", 0),
		News:         newsSource(context.Background()),
		Assist:       assist,
		KeyOverrides: keyOverrides(),
	})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	hl.Start(ctx)
	defer hl.Close()
	p := tea.NewProgram(model, tea.WithAltScreen())
	go drainEvents(p, hl.Events())
	startAssist(p, assist)
	_, err = p.Run()
	return err
}

// fetchSymbolList is the terminal's single /v1/symbols GET on the live
// startup path: displayConfig's precision lookup and watchInstruments'
// peer list both read from this one result instead of each fetching their
// own (TERM-STARTUP-DOUBLE-SYMBOLS-FETCH — the redundant second round trip
// doubled startup latency in live+streaming mode). A fetch failure is
// reported once here; both callers already fall back to their own defaults
// on an empty/nil list, so config discovery still never blocks startup.
func fetchSymbolList(gwURL string) []conn.SymbolConfig {
	symbols, err := conn.FetchSymbols(gwURL)
	if err != nil {
		fmt.Fprintf(os.Stderr, "rsx-term: symbol list fetch failed (%v); using defaults\n", err)
		return nil
	}
	return symbols
}

// symbolByID finds one symbol's config in an already-fetched list.
func symbolByID(symbols []conn.SymbolConfig, id uint32) (conn.SymbolConfig, bool) {
	for _, s := range symbols {
		if s.ID == id {
			return s, true
		}
	}
	return conn.SymbolConfig{}, false
}

// watchInstruments builds the streaming watchlist from an already-fetched
// symbol list: every symbol the gateway serves (decimals/tick from
// /v1/symbols) named/coded via RSX_TERM_WATCH ("id:name[:code]",
// comma-separated — the endpoint carries no names yet, so unnamed ids show
// as SYM-<id>). The primary symbol always leads. DOM mode (stream off)
// keeps the single-symbol list. symbols == nil (mock path, or a failed
// fetch) degrades to that single-symbol list too.
func watchInstruments(symbols []conn.SymbolConfig, primary uint32, priceDec, qtyDec int, tick int64, stream bool) []ui.Instrument {
	primaryIns := ui.Instrument{
		ID: primary, Name: Symbol, PriceDec: priceDec, QtyDec: qtyDec, Tick: tick,
	}
	if !stream {
		return []ui.Instrument{primaryIns}
	}
	names, codes := parseWatchEnv(os.Getenv("RSX_TERM_WATCH"))
	if n, ok := names[primary]; ok {
		primaryIns.Name = n
	}
	primaryIns.Code = codes[primary]
	out := []ui.Instrument{primaryIns}

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
		})
	}
	return out
}

// parseWatchEnv parses RSX_TERM_WATCH: "id:name[:code]" entries,
// comma-separated. Malformed entries are skipped (config sugar must never
// stop the terminal).
func parseWatchEnv(raw string) (names map[uint32]string, codes map[uint32]string) {
	names, codes = map[uint32]string{}, map[uint32]string{}
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
	}
	return names, codes
}

// displayConfig resolves the symbol's display precision + tick from an
// already-fetched symbol list (see fetchSymbolList — the live+streaming path
// fetches /v1/symbols exactly once and shares it with watchInstruments).
// symbols == nil (mock path) skips the lookup. A missing symbolID (fetch
// failed, or the gateway doesn't serve it) falls back to PENGU's values
// (price 6, qty 4, tick 1) with a note when the list itself is non-empty —
// config discovery must never keep the terminal from starting.
// RSX_TUI_PRICE_DECIMALS / RSX_TUI_QTY_DECIMALS / RSX_TUI_TICK, when set,
// override whatever base won.
func displayConfig(symbols []conn.SymbolConfig, symbolID uint32) (priceDec, qtyDec int, tick int64) {
	priceDec, qtyDec, tick = 6, 4, 1
	if cfg, ok := symbolByID(symbols, symbolID); ok {
		priceDec, qtyDec, tick = cfg.PriceDec, cfg.QtyDec, cfg.TickSize
	} else if len(symbols) > 0 {
		fmt.Fprintf(os.Stderr, "rsx-term: symbol %d not served; using defaults\n", symbolID)
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
	assist := assistSource()
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
		MaxNotional:  envI64("RSX_TERM_MAX_NOTIONAL", 0),
		News:         newsSource(context.Background()),
		Assist:       assist,
		KeyOverrides: keyOverrides(),
	})
	p := tea.NewProgram(model, tea.WithAltScreen())
	go feedDemo(p)
	startHL(p, hl)
	startAssist(p, assist)
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

	assist := assistSource()
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
		MaxNotional:  envI64("RSX_TERM_MAX_NOTIONAL", 0),
		News:         newsSource(ctx),
		Assist:       assist,
		KeyOverrides: keyOverrides(),
	})
	p := tea.NewProgram(model, tea.WithAltScreen())
	go drainEvents(p, live.Events())
	startHL(p, cfg.hl)
	startAssist(p, assist)
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
