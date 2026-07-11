// Package ui is the RSX trading terminal's Bubble Tea model: order-entry
// form, event folding over book / tape / position / latency state, key
// handling, and rendering. Mirrors the rsx-tui ratatui terminal
// (src/app.rs, input.rs, render.rs) and specs/2/55-terminal.md. All state
// lives in book.* (pure folds); this package adds the form, the message
// dispatch, and the lipgloss view.
package ui

import (
	"fmt"
	"strings"
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
	// Venue names the primary venue (default "rsx"). Sub and Instruments
	// above belong to it.
	Venue string
	// Venues are ADDITIONAL venues beyond the primary — the generic
	// multi-venue seam (e.g. read-only Hyperliquid market data). Their
	// tagged feed.VenueMsg events fold into per-venue markets.
	Venues []VenueConfig
	// MaxNotional is the streaming view's fat-finger ceiling in human quote
	// units per order — over it the order is hard-blocked. 0 → defaultMaxNotional.
	MaxNotional int64
	// News is the headline source (nil → news.Off, fully offline). The live
	// Tree of Alpha reader plugs in behind RSX_TERM_NEWS=1.
	News news.Source
	// KeyOverrides rebinds verb keys ({"action":"key"}, from RSX_TERM_KEYMAP).
	// An invalid map falls back to the defaults with a loud status.
	KeyOverrides map[string]string
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

// VenueConfig is one venue behind the generic-terminal seam: a name, its
// picker letter, its instrument universe, and its order Submitter (nil =
// read-only market data; trading there shows an honest block).
type VenueConfig struct {
	Name        string
	Code        string
	Instruments []Instrument
	Sub         feed.Submitter
}

// venueKey addresses one market: a symbol on a venue.
type venueKey struct {
	venue string
	id    uint32
}

// fmtPx / fmtQty render a raw price / qty as a human decimal using the
// ACTIVE instrument's precision — the one place raw i64 becomes a
// trader-readable number. With no watchlist (the DOM view) the active
// instrument is built from the legacy cfg fields, so nothing changes there.
func (m Model) fmtPx(raw int64) string  { return fmtDec(raw, m.ins().PriceDec) }
func (m Model) fmtQty(raw int64) string { return fmtDec(raw, m.ins().QtyDec) }

// ins returns the active instrument.
func (m Model) ins() Instrument { return m.instrumentFor(m.activeVenue, m.active) }

// instrumentFor resolves an instrument on a venue. The primary id falls back
// to the legacy single-symbol cfg fields; any other unknown id gets an honest
// SYM-<id> stub for the REQUESTED id (never the primary's name/scale).
func (m Model) instrumentFor(venue string, id uint32) Instrument {
	for _, v := range m.venues {
		if v.Name != venue {
			continue
		}
		for _, ins := range v.Instruments {
			if ins.ID == id {
				return ins
			}
		}
	}
	if id != m.cfg.SymbolID {
		// Unknown id (not on any configured venue): a stub carrying the requested
		// id, not the primary — otherwise an unwatched symbol renders under the
		// primary's name and price/qty scale. Precision is a best-effort default.
		return Instrument{
			ID:       id,
			Name:     fmt.Sprintf("SYM-%d", id),
			PriceDec: m.cfg.PriceDec,
			QtyDec:   m.cfg.QtyDec,
			Tick:     m.cfg.Tick,
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

// venueByName finds a configured venue (false for an unknown name).
func (m Model) venueByName(name string) (VenueConfig, bool) {
	for _, v := range m.venues {
		if v.Name == name {
			return v, true
		}
	}
	return VenueConfig{}, false
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

	// Streaming state (RSX_TERM_STREAM). Every watched (venue, symbol) folds
	// into its own market (book/tape/heatmap/persistence/position);
	// activeVenue+active name the one the book view renders. news feeds the
	// rail + news view (defaults to news.Off — always offline). heatW is the
	// heatmap's column count (0 until the first WindowSizeMsg).
	//
	// The watchlist model (lists / listSel / watchVenue) is the neutral
	// venue-markets seam the NEWS view reads and the BOOK switcher shares — it
	// is no longer a "pair" concept.
	keys        *keymap
	venues      []VenueConfig
	mkts        map[venueKey]*market
	activeVenue string
	active      uint32
	lastActive  map[string]uint32
	screen      screen
	heatW       int
	news        news.Source

	// Game order entry: the armed size preset (1-5, book view).
	sizeSel int

	// Book-view microscope: a keyboard row-cursor (↑/↓) over the rows the
	// heatmap already holds (Heatmap.Rows() — far + live, NOT the now row).
	// -1 = off. Freezing the cursor row hands it to the assistant. This is a
	// FREEZE of rows already in the ring, NOT a replay buffer: far rows are
	// aggregate time-weighted windows, never restored books.
	rowCursor int

	// Book-view symbol switcher (x + letter code) and the venue picker
	// (F9 + venue letter).
	switching    bool
	switchBuf    string
	venuePicking bool

	// Watchlist model: named venue-markets lists and the active one. Shared by
	// the NEWS view (its breadth venue) and the BOOK symbol switcher.
	lists   []watchlist
	listSel int

	// News view: feed selection, search state; the assistant handoff (the
	// packaged context + the instrument it was priced with).
	newsSel    int
	newsSearch bool
	newsQuery  string
	assistCtx  *news.AssistantContext
	assistIns  Instrument
}

// screen is which streaming view is on: the depth book (default), the news
// overview, or the LLM assistant.
type screen int

const (
	screenBook screen = iota
	screenNews
	screenLLM
)

// screenCount is how many screens the tab/shift+tab cycle rotates through.
const screenCount = 3

// label renders the screen's mode-line tag.
func (s screen) label() string {
	switch s {
	case screenNews:
		return "NEWS"
	case screenLLM:
		return "LLM"
	default:
		return "BOOK"
	}
}

// next / prev cycle the screens (tab / shift+tab): book → news → llm.
func (s screen) next() screen { return (s + 1) % screenCount }
func (s screen) prev() screen { return (s + screenCount - 1) % screenCount }

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
	if cfg.MaxNotional <= 0 {
		cfg.MaxNotional = defaultMaxNotional
	}
	if cfg.Venue == "" {
		cfg.Venue = "rsx"
	}
	assignCodes(cfg.Instruments)
	for i := range cfg.Venues {
		assignCodes(cfg.Venues[i].Instruments)
	}

	newsSrc := cfg.News
	if newsSrc == nil {
		newsSrc = news.Off{}
	}
	keys := defaultKeymap()
	keymapStatus := "connecting…"
	if len(cfg.KeyOverrides) > 0 {
		if err := keys.ApplyOverrides(cfg.KeyOverrides); err != nil {
			keys = defaultKeymap() // a broken keymap must not half-apply
			keymapStatus = "KEYMAP REJECTED: " + err.Error() + " — defaults active"
		}
	}
	m := Model{
		cfg:         cfg,
		keys:        keys,
		status:      keymapStatus,
		lastMdAgeNs: book.NsUnknown,
		news:        newsSrc,
		mkts:        map[venueKey]*market{},
		activeVenue: cfg.Venue,
		active:      cfg.SymbolID,
		lastActive:  map[string]uint32{},
		rowCursor:   -1,
	}
	primary := VenueConfig{Name: cfg.Venue, Code: venuePickCode(cfg.Venue), Instruments: cfg.Instruments, Sub: cfg.Sub}
	m.venues = []VenueConfig{primary}
	m.venues = append(m.venues, cfg.Venues...)

	// Extra (breadth) venues list first: the pair/news screens default to
	// the widest universe (e.g. Hyperliquid) when one is configured.
	for i := len(m.venues) - 1; i >= 0; i-- {
		v := m.venues[i]
		ids := make([]uint32, 0, len(v.Instruments))
		for _, ins := range v.Instruments {
			m.mkts[venueKey{v.Name, ins.ID}] = newMarket(ins)
			ids = append(ids, ins.ID)
		}
		m.lists = append(m.lists, watchlist{name: v.Name, venue: v.Name, ids: ids})
	}
	return m
}

// venuePickCode is a venue's F9-picker letter: its first letter.
func venuePickCode(name string) string {
	if name == "" {
		return "?"
	}
	return strings.ToLower(name[:1])
}

// mkt returns the active market (creating it defensively for an unknown id).
func (m Model) mkt() *market { return m.marketFor(m.activeVenue, m.active) }

// marketFor returns the market for a symbol on a venue, creating one on
// first sight (an unsubscribed frame or a fill on a symbol outside the
// watchlist must not crash the fold).
func (m Model) marketFor(venue string, id uint32) *market {
	key := venueKey{venue, id}
	if mk, ok := m.mkts[key]; ok {
		return mk
	}
	mk := newMarket(m.instrumentFor(venue, id))
	m.mkts[key] = mk
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
