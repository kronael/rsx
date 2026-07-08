// Package ui is the RSX trading terminal's Bubble Tea model: order-entry
// form, event folding over book / tape / position / latency state, key
// handling, and rendering. Mirrors the rsx-tui ratatui terminal
// (src/app.rs, input.rs, render.rs) and specs/2/55-terminal.md. All state
// lives in book.* (pure folds); this package adds the form, the message
// dispatch, and the lipgloss view.
package ui

import (
	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/feed"
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

	book     book.Book
	seq      book.SeqTracker
	tape     book.Tape
	position book.Position

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
	fills          int

	lastLat   *book.Sample
	latWindow book.Window
	showTrace bool

	width  int
	height int
}

// New builds a fresh model. Zero-value book / seq / tape / position /
// latWindow are ready to fold; side defaults to Buy, tif to GTC, focus to the
// price field (all the useful zero values).
func New(cfg Config) Model {
	return Model{
		cfg:    cfg,
		status: "connecting…",
	}
}

// Init satisfies tea.Model. The live/mock feeds are driven externally
// (main.go), so there is no startup command.
func (m Model) Init() tea.Cmd { return nil }

// Position returns the client-derived position. Exported so external tests
// (and a future account panel) can read the folded net / entry / uPnL without
// reaching into unexported state.
func (m Model) Position() book.Position { return m.position }
