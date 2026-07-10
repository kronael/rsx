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
}

// fmtPx / fmtQty render a raw price / qty as a human decimal using the
// symbol's configured precision — the one place raw i64 becomes a
// trader-readable number.
func (m Model) fmtPx(raw int64) string  { return fmtDec(raw, m.cfg.PriceDec) }
func (m Model) fmtQty(raw int64) string { return fmtDec(raw, m.cfg.QtyDec) }

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

// OpenOrder is a resting order this session submitted, tracked so 'c' can
// cancel the newest and the status bar can count them.
type OpenOrder struct {
	Oid  uint64
	Cid  string
	Side wire.Side
	Px   int64
	Qty  int64
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

	// Streaming heatmap state (RSX_TERM_STREAM). heat is nil until the first
	// WindowSizeMsg sizes the ring; pendingTrades accumulates prints between bin
	// ticks; news feeds the left rail (defaults to news.Off — always offline).
	heat          *book.Heatmap
	pendingTrades []book.TapeEntry
	news          news.Source
}

// New builds a fresh model. Zero-value book / seq / tape / position /
// latWindow are ready to fold; side defaults to Buy, tif to GTC, focus to the
// price field (all the useful zero values).
func New(cfg Config) Model {
	return Model{
		cfg:         cfg,
		status:      "connecting…",
		lastMdAgeNs: book.NsUnknown,
		news:        news.Off{},
	}
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
