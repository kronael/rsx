// Package ui is the RSX trading terminal's Bubble Tea model: order-entry
// form, event folding over book / tape / position / latency state, key
// handling, and rendering. Mirrors the rsx-tui ratatui terminal
// (src/app.rs, input.rs, render.rs) and specs/2/55-terminal.md. All state
// lives in book.* (pure folds); this package adds the form, the message
// dispatch, and the lipgloss view.
package ui

import (
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/feed"
	"rsx-term/news"
	"rsx-term/wire"
)

// Config is the terminal's static configuration: the single symbol it trades,
// the endpoints (for the trace HUD), and the order Submitter.
type Config struct {
	Symbol     string
	SymbolID   uint32
	Endpoint   string
	MdEndpoint string
	Sub        feed.Submitter
	// PriceDec / QtyDec convert raw i64 px/qty to human decimals at display
	// (raw / 10^dec). PENGU is 6 / 4 (a ~$0.01 symbol: raw 10001 = 0.010001);
	// 0 shows raw. Source: the symbol's price_decimals / qty_decimals.
	PriceDec int
	QtyDec   int
	// Tick is the smallest raw price increment (`+`/`-` step this much). PENGU
	// is 1; 0 falls back to 1 so the nudge keys always move by something.
	Tick int64
	// Stream selects the streaming "text Bookmap" heatmap view (RSX_TERM_STREAM=1).
	// Off (the default) renders the classic DOM three-column view, unchanged.
	Stream bool
	// SizePresets are the five game-entry order sizes (raw qty) the 1-5 keys
	// arm in the streaming view. Empty falls back to defaultSizePresets.
	SizePresets []int64
	// Instruments is the streaming watchlist (first entry = the primary
	// symbol). Empty falls back to the single legacy Symbol/SymbolID.
	Instruments []Instrument
	// LotNotional is the pair view's base lot size in HUMAN quote units
	// (1 lot ≈ LotNotional × instrument.LotMult% of notional). 0 → 100.
	LotNotional int64
	// MaxNotional is the pair view's fat-finger ceiling in human quote units
	// per order — over it the order is hard-blocked. 0 → 100 × LotNotional.
	MaxNotional int64
}

// defaultSizePresets builds the streaming view's stock size ladder — 1, 2, 5,
// 10, 25 whole units at the symbol's qty precision.
func defaultSizePresets(qtyDec int) []int64 {
	unit := pow10(qtyDec)
	return []int64{1 * unit, 2 * unit, 5 * unit, 10 * unit, 25 * unit}
}

// sizePreset is the currently armed game-entry size (raw qty): an explicit
// cfg override, else the stock ladder at the ACTIVE instrument's precision
// (so a symbol hop keeps presets meaningful).
func (m Model) sizePreset() int64 {
	presets := m.cfg.SizePresets
	if len(presets) == 0 {
		presets = defaultSizePresets(m.ins().QtyDec)
	}
	sel := clamp(m.sizeSel, 0, len(presets)-1)
	return presets[sel]
}

// fmtPx / fmtQty render a raw price / qty as a human decimal using the
// ACTIVE instrument's precision — the one place raw i64 becomes a
// trader-readable number. With no watchlist (the DOM view) the active
// instrument is built from the legacy cfg fields, so nothing changes there.
func (m Model) fmtPx(raw int64) string  { return fmtDec(raw, m.ins().PriceDec) }
func (m Model) fmtQty(raw int64) string { return fmtDec(raw, m.ins().QtyDec) }

// ins returns the active instrument.
func (m Model) ins() Instrument { return m.instrumentFor(m.active) }

// instrumentFor resolves an instrument by symbol id, falling back to the
// legacy single-symbol cfg fields for the primary (or an unknown) id.
func (m Model) instrumentFor(id uint32) Instrument {
	for _, ins := range m.cfg.Instruments {
		if ins.ID == id {
			return ins
		}
	}
	return Instrument{
		ID:       m.cfg.SymbolID,
		Name:     m.cfg.Symbol,
		PriceDec: m.cfg.PriceDec,
		QtyDec:   m.cfg.QtyDec,
		Tick:     m.cfg.Tick,
	}
}

// fmtNotional renders a raw price×qty product (notional, uPnL) as money in the
// quote currency. The raw product carries price_dec+qty_dec of scale, but a
// money figure reads at the *quote's* precision (price_dec) — showing all
// price_dec+qty_dec digits tacks on qty_dec meaningless trailing zeros
// ($0.0500050000). So divide out the qty scale (10^qty_dec) to land back at
// price-scale, then format at price_dec. Integer division truncates toward
// zero, which is the right rounding for a sub-precision money remainder.
func (m Model) fmtNotional(raw int64) string {
	scale := int64(1)
	for i := 0; i < m.cfg.QtyDec; i++ {
		scale *= 10
	}
	return fmtDec(raw/scale, m.cfg.PriceDec)
}

// Focus is which order-entry field the digit keys edit.
type Focus int

const (
	// FocusPx edits the price buffer.
	FocusPx Focus = iota
	// FocusQty edits the quantity buffer.
	FocusQty
)

// OpenOrder is a resting order this session submitted, tracked so 'c'/'d'
// can cancel it and the status bar can count them. Symbol routes multi-symbol
// sessions (always the resolved id, never 0).
type OpenOrder struct {
	Oid    uint64
	Cid    string
	Side   wire.Side
	Px     int64
	Qty    int64
	Symbol uint32
}

// Model is the whole terminal state. It satisfies tea.Model
// (Init / Update / View).
type Model struct {
	cfg Config

	book         book.Book
	seq          book.SeqTracker
	tape         book.Tape
	position     book.Position
	ladderCenter int64 // static-ladder centre price (0 = uninitialised)

	gwConnected bool
	mdConnected bool
	status      string

	// Order-entry form.
	side       wire.Side
	pxBuf      string
	qtyBuf     string
	tif        wire.Tif
	reduceOnly bool
	postOnly   bool
	focus      Focus

	pendingConfirm *wire.OrderReq
	openOrders     []OpenOrder // newest last
	orderSel       int         // selection cursor into openOrders (for `c`)
	fills          int

	lastLat   *book.Sample
	latWindow book.Window
	showTrace bool
	showHelp  bool
	// armed = confirm-off: orders fire on a single enter (no two-step preview).
	// A loud banner warns while it's on; the fat-finger size guard still holds.
	armed bool

	// Marketdata-path telemetry: client-measured age of the most recent
	// md frame (wall-clock now minus the frame's server ts_ns) and when it
	// last arrived, for staleness. Real numbers, not placeholders — every
	// md frame carries ts_ns (specs/2/49-webproto.md), so this needs no
	// server change. A frame with ts_ns == 0 (the offline demo script
	// doesn't stamp one) is not measurable and stays book.NsUnknown rather
	// than showing a fabricated multi-decade age.
	lastMdAgeNs int64
	mdAgeWindow book.Window
	lastMdAt    time.Time

	width  int
	height int

	// Streaming state (RSX_TERM_STREAM). Every watched symbol folds into its
	// own market (book/tape/heatmap/persistence/position); active names the
	// one the book view renders. news feeds the rail + news view (defaults
	// to news.Off — always offline). heatW is the heatmap's column count
	// (0 until the first WindowSizeMsg).
	mkts   map[uint32]*market
	active uint32
	screen screen
	heatW  int
	news   news.Source

	// Game order entry: the armed size preset (1-5, book view).
	sizeSel int

	// Book-view symbol switcher (x + letter code).
	switching bool
	switchBuf string

	// Pair view: named watchlists, the armed symbol (0 = none), and the
	// vim-count buffer (digits before b/s).
	lists    []watchlist
	listSel  int
	armedSym uint32
	countBuf string
}

// screen is which streaming view is on: the depth book (default), the
// multi-pair chase grid, the news feed, or the LLM assistant.
type screen int

const (
	screenBook screen = iota
	screenPair
	screenNews
	screenLLM
)

// label renders the screen's mode-line tag.
func (s screen) label() string {
	switch s {
	case screenPair:
		return "PAIR"
	case screenNews:
		return "NEWS"
	case screenLLM:
		return "LLM"
	default:
		return "BOOK"
	}
}

// next / prev cycle the four screens (tab / shift+tab).
func (s screen) next() screen { return (s + 1) % 4 }
func (s screen) prev() screen { return (s + 3) % 4 }

// New builds a fresh model. Zero-value book / seq / tape / position /
// latWindow are ready to fold; side defaults to Buy, tif to GTC, focus to the
// price field (all the useful zero values). The streaming watchlist gets its
// switcher codes assigned here and every instrument gets a market.
func New(cfg Config) Model {
	if len(cfg.Instruments) == 0 {
		cfg.Instruments = []Instrument{{
			ID:       cfg.SymbolID,
			Name:     cfg.Symbol,
			PriceDec: cfg.PriceDec,
			QtyDec:   cfg.QtyDec,
			Tick:     cfg.Tick,
		}}
	}
	if cfg.LotNotional <= 0 {
		cfg.LotNotional = 100
	}
	if cfg.MaxNotional <= 0 {
		cfg.MaxNotional = 100 * cfg.LotNotional
	}
	assignCodes(cfg.Instruments)

	m := Model{
		cfg:         cfg,
		status:      "connecting…",
		lastMdAgeNs: book.NsUnknown,
		news:        news.Off{},
		mkts:        map[uint32]*market{},
		active:      cfg.SymbolID,
	}
	ids := make([]uint32, 0, len(cfg.Instruments))
	for _, ins := range cfg.Instruments {
		m.mkts[ins.ID] = newMarket(ins)
		ids = append(ids, ins.ID)
	}
	m.lists = []watchlist{{name: "all", ids: ids}}
	return m
}

// mkt returns the active market (creating it defensively for an unknown id).
func (m Model) mkt() *market { return m.marketFor(m.active) }

// marketFor returns the market for a symbol id, creating one on first sight
// (an unsubscribed frame or a fill on a symbol outside the watchlist must
// not crash the fold).
func (m Model) marketFor(id uint32) *market {
	if mk, ok := m.mkts[id]; ok {
		return mk
	}
	mk := newMarket(m.instrumentFor(id))
	m.mkts[id] = mk
	return mk
}

// Init satisfies tea.Model. The live/mock feeds are driven externally
// (main.go), so the only startup command is the streaming heatmap's bin tick
// (DOM mode has none — it returns nil, unchanged).
func (m Model) Init() tea.Cmd {
	if m.cfg.Stream {
		return binTickCmd()
	}
	return nil
}

// Position returns the client-derived position. Exported so external tests
// (and a future account panel) can read the folded net / entry / uPnL without
// reaching into unexported state.
func (m Model) Position() book.Position { return m.position }
