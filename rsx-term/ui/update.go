package ui

import (
	"fmt"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/book"
	"rsx-term/feed"
	"rsx-term/wire"
)

// digitCap bounds each digit buffer so it always parses into an int64.
const digitCap = 18

// Update folds one message into the model. Key messages drive the order form;
// wire.* / feed.* messages fold market data, private events, and link state.
func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch v := msg.(type) {
	case tea.KeyMsg:
		if m.cfg.Stream {
			return m.handleStreamKey(v)
		}
		return m.handleKey(v)
	case tea.MouseMsg:
		if m.cfg.Stream {
			return m.handleStreamMouse(v)
		}
		return m.handleMouse(v)

	case feed.GwUp:
		m.gwConnected = true
		m.status = "connected"
	case feed.GwDown:
		m.gwConnected = false
		m.status = "disconnected"
	case feed.MdUp:
		m.mdConnected = true
	case feed.MdDown:
		m.mdConnected = false

	case wire.Snapshot:
		if m.primaryFrame(v.SymbolID) {
			m.seq.ResetTo(v.Seq)
			m.book.ApplySnapshot(v)
			m.recenterLadder()
			m.foldMdFrame(v.TsNs)
		}
		if m.cfg.Stream {
			mk := m.marketFor(m.frameSymbol(v.SymbolID))
			mk.book.ApplySnapshot(v)
			mk.persist.ObserveSnapshot(v.Bids, v.Asks, time.Now().UnixNano())
		}
	case wire.Delta:
		if m.primaryFrame(v.SymbolID) {
			m.seq.Observe(v.Seq)
			m.book.ApplyDelta(v)
			m.recenterLadder()
			m.foldMdFrame(v.TsNs)
		}
		if m.cfg.Stream {
			mk := m.marketFor(m.frameSymbol(v.SymbolID))
			mk.book.ApplyDelta(v)
			mk.persist.ObserveDelta(v, time.Now().UnixNano())
		}
	case wire.Bbo:
		if m.primaryFrame(v.SymbolID) {
			m.seq.Observe(v.Seq)
			m.book.ApplyBbo(v)
			m.recenterLadder()
			m.foldMdFrame(v.TsNs)
		}
		if m.cfg.Stream {
			m.marketFor(m.frameSymbol(v.SymbolID)).book.ApplyBbo(v)
		}
	case wire.MdTrade:
		side := wire.Buy
		if v.TakerSide != 0 {
			side = wire.Sell
		}
		entry := book.TapeEntry{Side: side, Px: v.Px, Qty: v.Qty}
		if m.primaryFrame(v.SymbolID) {
			m.seq.Observe(v.Seq)
			m.tape.Push(entry)
			m.foldMdFrame(v.TsNs)
		}
		if m.cfg.Stream {
			mk := m.marketFor(m.frameSymbol(v.SymbolID))
			mk.tape.Push(entry)
			mk.pending = append(mk.pending, entry)
		}

	case wire.Accepted:
		m.openOrders = append(m.openOrders, OpenOrder{
			Oid:    v.Oid,
			Cid:    v.Cid,
			Side:   v.Order.Side,
			Px:     v.Order.Px,
			Qty:    v.Order.Qty,
			Symbol: m.frameSymbol(v.Order.Symbol),
		})
		m.status = fmt.Sprintf("order %d accepted", v.Oid)
		m.foldRtt(v.RttNs)
	case wire.Done:
		m.removeOrder(v.Oid)
		m.status = fmt.Sprintf("order %d done", v.Oid)
		m.foldRtt(v.RttNs)
	case wire.Rejected:
		m.status = "rejected: " + v.Reason
	case wire.Fill:
		m.fills++
		if m.primaryFrame(v.Symbol) {
			m.position.ApplyFill(v.Side, v.Px, v.Qty)
		}
		if m.cfg.Stream {
			m.marketFor(m.frameSymbol(v.Symbol)).position.ApplyFill(v.Side, v.Px, v.Qty)
		}
		m.status = fmt.Sprintf("fill %d: %s @ %s", v.Oid, m.fmtQty(v.Qty), m.fmtPx(v.Px))

	case feed.Latency:
		s := v.Sample
		m.lastLat = &s
		m.latWindow.Add(v.Sample.TotalNs)

	case tea.WindowSizeMsg:
		m.width = v.Width
		m.height = v.Height
		if m.cfg.Stream {
			m.resizeHeat()
		}

	case binTickMsg:
		if m.cfg.Stream {
			m.foldBin(time.Time(v).UnixNano())
			return m, binTickCmd()
		}
	}
	return m, nil
}

// handleStreamKey routes a key by the active screen. Global chrome first
// (quit, help, tab view-cycle, r/p modifier toggles — persistent modes shown
// on the top mode line); then the screen's own grammar. Orders fire on ONE
// keypress everywhere in the streaming terminal (no two-enter confirm — the
// decided design); the fat-finger caps still hard-block in fire.
func (m Model) handleStreamKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	key := k.String()
	if m.showHelp {
		if key == "q" || key == "ctrl+c" {
			return m, tea.Quit
		}
		m.showHelp = false
		return m, nil
	}
	if m.switching {
		return m.handleSwitchKey(key)
	}
	switch key {
	case "q", "ctrl+c":
		return m, tea.Quit
	case "?":
		m.showHelp = true
		return m, nil
	case "tab":
		m.screen = m.screen.next()
		return m, nil
	case "shift+tab":
		m.screen = m.screen.prev()
		return m, nil
	case "r":
		m.reduceOnly = !m.reduceOnly
		m.status = "reduce-only " + onOff(m.reduceOnly) + " (applies to every order until toggled)"
		return m, nil
	case "p":
		m.postOnly = !m.postOnly
		m.status = "post-only " + onOff(m.postOnly) + " (applies to resting orders until toggled)"
		return m, nil
	}
	switch m.screen {
	case screenPair:
		return m.handlePairKey(key)
	case screenNews:
		return m.handleNewsKey(key)
	case screenLLM:
		return m.handleLLMKey(key)
	default:
		return m.handleBookKey(key)
	}
}

// handleBookKey is the depth/book view's game order entry: size presets, a
// price cursor, single-key place/cancel, single-key crosses, and the x
// symbol switcher.
func (m Model) handleBookKey(key string) (tea.Model, tea.Cmd) {
	switch key {
	case "esc":
		return m, tea.Quit
	case "x":
		m.switching = true
		m.switchBuf = ""
	case "b":
		m.side = wire.Buy
	case "s":
		m.side = wire.Sell
	case "1", "2", "3", "4", "5":
		m.sizeSel = int(key[0] - '1')
		m.status = fmt.Sprintf("size %s armed", m.fmtQty(m.sizePreset()))
	case "!", "@", "#", "$", "%":
		return m.handleCross(shiftDigitSel(key))
	case "h", "left":
		m.stepCursor(-1)
	case "l", "right":
		m.stepCursor(+1)
	case "j", "down":
		m.cursorToTouch(wire.Buy)
	case "k", "up":
		m.cursorToTouch(wire.Sell)
	case "f":
		return m.handlePlace()
	case "d":
		return m.handleStreamCancel()
	}
	return m, nil
}

// handleSwitchKey is the book view's rapid symbol switcher: x, then the
// symbol's letter code (shown in the mode line). Exact code match switches
// instantly; esc cancels; backspace edits.
func (m Model) handleSwitchKey(key string) (tea.Model, tea.Cmd) {
	switch {
	case key == "esc" || key == "x":
		m.switching = false
		m.switchBuf = ""
	case key == "backspace":
		if len(m.switchBuf) > 0 {
			m.switchBuf = m.switchBuf[:len(m.switchBuf)-1]
		}
	case len(key) == 1 && key[0] >= 'a' && key[0] <= 'z':
		m.switchBuf += key
		if ins, ok := m.instrumentByCode(m.switchBuf); ok {
			m.switchTo(ins)
			return m, nil
		}
		if len(m.switchBuf) >= 2 {
			m.status = fmt.Sprintf("no symbol coded %q", m.switchBuf)
			m.switching = false
			m.switchBuf = ""
		}
	}
	return m, nil
}

// instrumentByCode finds a watchlist instrument by its switcher code.
func (m Model) instrumentByCode(code string) (Instrument, bool) {
	for _, ins := range m.cfg.Instruments {
		if ins.Code == code {
			return ins, true
		}
	}
	return Instrument{}, false
}

// switchTo makes ins the active symbol: the book view re-anchors onto its
// market's heatmap (already folding in the background, so the hop is
// instant).
func (m *Model) switchTo(ins Instrument) {
	m.active = ins.ID
	m.switching = false
	m.switchBuf = ""
	m.status = "book → " + ins.Name
}

// shiftDigitSel maps the shifted digit row (!@#$%) back to preset index 0-4.
func shiftDigitSel(key string) int {
	switch key {
	case "!":
		return 0
	case "@":
		return 1
	case "#":
		return 2
	case "$":
		return 3
	default:
		return 4
	}
}

// stepCursor nudges the active market's price cursor n ticks, seeding it
// from the mid the first time. Floored at one tick.
func (m *Model) stepCursor(n int64) {
	mk := m.mkt()
	t := mk.ins.Tick
	if t <= 0 {
		t = 1
	}
	if mk.cursorPx == 0 {
		if mid, ok := mk.book.Mid(); ok {
			mk.cursorPx = (mid / t) * t
		} else {
			mk.cursorPx = t
		}
	}
	mk.cursorPx += n * t
	if mk.cursorPx < t {
		mk.cursorPx = t
	}
}

// cursorToTouch snaps the cursor to the touch: j → best bid, k → best ask.
func (m *Model) cursorToTouch(side wire.Side) {
	mk := m.mkt()
	if side == wire.Buy {
		if b, ok := mk.book.BestBid(); ok {
			mk.cursorPx = b.Px
			return
		}
		m.status = "no bid to join"
		return
	}
	if a, ok := mk.book.BestAsk(); ok {
		mk.cursorPx = a.Px
		return
	}
	m.status = "no ask to join"
}

// handlePlace fires a resting limit at the cursor (or, unset, the side's own
// touch): the quoting keystroke. Side b/s, size = the armed preset, GTC.
func (m Model) handlePlace() (tea.Model, tea.Cmd) {
	mk := m.mkt()
	px := mk.cursorPx
	if px == 0 {
		if m.side == wire.Buy {
			b, ok := mk.book.BestBid()
			if !ok {
				m.status = "place: no bid to join (move the cursor first)"
				return m, nil
			}
			px = b.Px
		} else {
			a, ok := mk.book.BestAsk()
			if !ok {
				m.status = "place: no ask to join (move the cursor first)"
				return m, nil
			}
			px = a.Px
		}
	}
	return m.fire(wire.OrderReq{Symbol: m.active, Side: m.side, Px: px, Qty: m.sizePreset(), Tif: wire.Gtc})
}

// handleCross fires an aggressive IOC of preset sel at the far touch — the
// hit/lift keystroke (shift+1-5). Buy crosses the best ask, sell the best bid.
func (m Model) handleCross(sel int) (tea.Model, tea.Cmd) {
	m.sizeSel = clamp(sel, 0, 4)
	mk := m.mkt()
	var px int64
	if m.side == wire.Buy {
		a, ok := mk.book.BestAsk()
		if !ok {
			m.status = "cross: no ask to lift"
			return m, nil
		}
		px = a.Px
	} else {
		b, ok := mk.book.BestBid()
		if !ok {
			m.status = "cross: no bid to hit"
			return m, nil
		}
		px = b.Px
	}
	return m.fire(wire.OrderReq{Symbol: m.active, Side: m.side, Px: px, Qty: m.sizePreset(), Tif: wire.Ioc})
}

// handleStreamCancel cancels the own resting order on the active symbol
// nearest the cursor (the point-and-delete of game entry); with no cursor it
// cancels the newest.
func (m Model) handleStreamCancel() (tea.Model, tea.Cmd) {
	own := m.ownOrdersFor(m.active)
	if len(own) == 0 {
		m.status = "no open order to cancel"
		return m, nil
	}
	idx := len(own) - 1
	if cursor := m.mkt().cursorPx; cursor > 0 {
		best := int64(-1)
		for i, o := range own {
			d := o.Px - cursor
			if d < 0 {
				d = -d
			}
			if best < 0 || d < best {
				best, idx = d, i
			}
		}
	}
	o := own[idx]
	if o.Cid == "" {
		m.status = "selected order has no cid yet"
		return m, nil
	}
	if err := m.cfg.Sub.Cancel(o.Cid); err != nil {
		m.status = "cancel failed: " + err.Error()
	} else {
		m.status = fmt.Sprintf("cancel sent for order %d @ %s", o.Oid, m.fmtPx(o.Px))
	}
	return m, nil
}

// ownOrdersFor filters this session's working orders to one symbol.
func (m Model) ownOrdersFor(id uint32) []OpenOrder {
	var out []OpenOrder
	for _, o := range m.openOrders {
		if m.frameSymbol(o.Symbol) == id {
			out = append(out, o)
		}
	}
	return out
}

// handlePairKey is the pair view's chase/directional grammar: a symbol
// LETTER arms its row; an optional digit count then `b`/`s` fires that many
// lots at market (IOC, aggressive is the default here); `.` flattens
// (reduce-only); `[`/`]` switch watchlists; esc clears the arm. No passive
// quoting in this view — that is the book view's job.
func (m Model) handlePairKey(key string) (tea.Model, tea.Cmd) {
	switch {
	case key == "esc":
		m.armedSym = 0
		m.countBuf = ""
	case key == "[" || key == "]":
		n := len(m.lists)
		if key == "[" {
			m.listSel = (m.listSel + n - 1) % n
		} else {
			m.listSel = (m.listSel + 1) % n
		}
		m.armedSym = 0
		m.countBuf = ""
		m.status = "list " + m.lists[m.listSel].name
	case len(key) == 1 && key[0] >= '0' && key[0] <= '9':
		if m.armedSym != 0 && len(m.countBuf) < 2 {
			m.countBuf += key
		}
	case key == "b":
		return m.handlePairTrade(wire.Buy)
	case key == "s":
		return m.handlePairTrade(wire.Sell)
	case key == ".":
		return m.handlePairFlatten()
	case len(key) == 1 && key[0] >= 'a' && key[0] <= 'z':
		if ins, ok := m.instrumentByCode(key); ok && m.inActiveList(ins.ID) {
			m.armedSym = ins.ID
			m.countBuf = ""
			m.status = "armed " + ins.Name
		}
	}
	return m, nil
}

// inActiveList reports whether a symbol is on the active watchlist.
func (m Model) inActiveList(id uint32) bool {
	for _, lid := range m.lists[m.listSel].ids {
		if lid == id {
			return true
		}
	}
	return false
}

// lots parses the vim-count buffer (digits typed before b/s), defaulting 1.
func (m Model) lots() int64 {
	n, ok := parseDigits(m.countBuf)
	if !ok || n < 1 {
		return 1
	}
	return n
}

// handlePairTrade fires `lots` lots at market on the armed symbol: buy lifts
// the offer, sell hits the bid. IOC always — the pair view is aggressive by
// design. The notional fat-finger cap hard-blocks in fire.
func (m Model) handlePairTrade(side wire.Side) (tea.Model, tea.Cmd) {
	if m.armedSym == 0 {
		m.status = "arm a symbol first (its letter key)"
		return m, nil
	}
	mk := m.marketFor(m.armedSym)
	var px int64
	if side == wire.Buy {
		a, ok := mk.book.BestAsk()
		if !ok {
			m.status = "no ask to lift on " + mk.ins.Name
			return m, nil
		}
		px = a.Px
	} else {
		b, ok := mk.book.BestBid()
		if !ok {
			m.status = "no bid to hit on " + mk.ins.Name
			return m, nil
		}
		px = b.Px
	}
	lotQty, ok := m.lotQty(mk.ins, px)
	if !ok {
		m.status = "can't size a lot on " + mk.ins.Name
		return m, nil
	}
	qty, ok := safeMul(lotQty, m.lots())
	if !ok {
		m.status = "BLOCKED: lot count overflows"
		return m, nil
	}
	m.countBuf = ""
	return m.fire(wire.OrderReq{Symbol: m.armedSym, Side: side, Px: px, Qty: qty, Tif: wire.Ioc})
}

// handlePairFlatten fires the reduce-only IOC that closes the armed symbol's
// derived position at its far touch (`.`).
func (m Model) handlePairFlatten() (tea.Model, tea.Cmd) {
	if m.armedSym == 0 {
		m.status = "arm a symbol first (its letter key)"
		return m, nil
	}
	mk := m.marketFor(m.armedSym)
	net := mk.position.Net
	if net == 0 {
		m.status = "no position on " + mk.ins.Name
		return m, nil
	}
	qty := net
	side := wire.Sell
	if net < 0 {
		qty, side = -net, wire.Buy
	}
	var px int64
	if side == wire.Sell {
		b, ok := mk.book.BestBid()
		if !ok {
			m.status = "no bid to hit on " + mk.ins.Name
			return m, nil
		}
		px = b.Px
	} else {
		a, ok := mk.book.BestAsk()
		if !ok {
			m.status = "no ask to lift on " + mk.ins.Name
			return m, nil
		}
		px = a.Px
	}
	return m.fire(wire.OrderReq{Symbol: m.armedSym, Side: side, Px: px, Qty: qty, Tif: wire.Ioc, ReduceOnly: true})
}

// lotQty sizes ONE lot of an instrument at a price: the configured base
// notional (human quote units) scaled by the instrument's multiplier — a
// consistent risk unit per symbol, in raw qty. Integer math throughout.
func (m Model) lotQty(ins Instrument, px int64) (int64, bool) {
	if px <= 0 {
		return 0, false
	}
	scaled, ok := safeMul(m.cfg.LotNotional, pow10(ins.PriceDec+ins.QtyDec))
	if !ok {
		return 0, false
	}
	qty := scaled / px
	mult := ins.LotMult
	if mult <= 0 {
		mult = 100
	}
	qty, ok = safeMul(qty, mult)
	if !ok {
		return 0, false
	}
	qty /= 100
	if qty < 1 {
		qty = 1
	}
	return qty, true
}

// pow10 is 10^n as i64 (n small, display-precision scale).
func pow10(n int) int64 {
	out := int64(1)
	for i := 0; i < n; i++ {
		out *= 10
	}
	return out
}

// handleNewsKey / handleLLMKey — the news and assistant views' keys (esc
// returns to the book; their own grammars live with their views).
func (m Model) handleNewsKey(key string) (tea.Model, tea.Cmd) {
	if key == "esc" {
		m.screen = screenBook
	}
	return m, nil
}

func (m Model) handleLLMKey(key string) (tea.Model, tea.Cmd) {
	if key == "esc" {
		m.screen = screenBook
	}
	return m, nil
}

// fire applies the fat-finger guards and submits immediately — the streaming
// terminal's single-keypress path. Guards: the hard qty cap (maxOrderQty,
// same as the DOM confirm path) and the notional ceiling (MaxNotional, human
// quote units). Over either, the order is BLOCKED outright. The persistent
// modifier toggles (r/p, shown on the mode line) apply to every order here:
// reduce-only to all, post-only to resting (GTC) orders.
func (m Model) fire(o wire.OrderReq) (tea.Model, tea.Cmd) {
	ins := m.instrumentFor(m.frameSymbol(o.Symbol))
	if qtyCap, ok := safeMul(maxOrderQty, pow10(ins.QtyDec)); ok && o.Qty > qtyCap {
		// The DOM cap is raw units for the primary symbol; here it scales to
		// the instrument's precision (1e6 WHOLE units) — the notional ceiling
		// below is the guard that actually bites first.
		m.status = fmt.Sprintf("BLOCKED: qty %s exceeds max (fat-finger guard)", m.fmtQty(o.Qty))
		return m, nil
	}
	if notional, ok := safeMul(o.Px, o.Qty); ok {
		ceiling, ceilOk := safeMul(m.cfg.MaxNotional, pow10(ins.PriceDec+ins.QtyDec))
		if ceilOk && notional > ceiling {
			m.status = fmt.Sprintf("BLOCKED: notional over %d (fat-finger guard)", m.cfg.MaxNotional)
			return m, nil
		}
	}
	o.ReduceOnly = o.ReduceOnly || m.reduceOnly
	if o.Tif == wire.Gtc {
		o.PostOnly = o.PostOnly || m.postOnly
	}
	if err := m.cfg.Sub.Submit(o); err != nil {
		m.status = "submit failed: " + err.Error()
		return m, nil
	}
	m.status = m.sentStatus(o)
	return m, nil
}

// primaryFrame reports whether a frame belongs to the legacy single-symbol
// state (the DOM view's book/tape/position): the configured symbol, or 0 —
// an unspecified id from the mock/demo and older tests.
func (m Model) primaryFrame(id uint32) bool {
	return id == 0 || id == m.cfg.SymbolID
}

// frameSymbol resolves a frame's symbol id, mapping the unspecified 0 to the
// primary symbol.
func (m Model) frameSymbol(id uint32) uint32 {
	if id == 0 {
		return m.cfg.SymbolID
	}
	return id
}

// foldBin closes the open time bin at nowNs for EVERY watched market: each
// folds its live book, its pending trades, and its level ages into a fresh
// live row, advances its stable ramp bases and pair-view reads, then clears
// its pending buffer. No-op per market until the grid is sized.
func (m *Model) foldBin(nowNs int64) {
	for _, mk := range m.mkts {
		mk.foldBinAt(nowNs)
	}
}

// foldBinAt is one market's bin fold (see foldBin).
func (mk *market) foldBinAt(nowNs int64) {
	if mk.heat.LiveCap() == 0 {
		return // not sized yet
	}
	if mk.lastBinNs == 0 {
		mk.lastBinNs = nowNs - int64(binInterval)
	}
	mk.heat.Ingest(mk.book.Bids, mk.book.Asks, mk.pending, mk.persist, mk.lastBinNs, nowNs)
	mk.foldBases()
	mk.foldPairReads()
	mk.lastBinNs = nowNs
	mk.pending = mk.pending[:0]
}

// basisDecayShift decays each ramp basis by 1/256 per bin (~18s half-life at
// 100ms bins) — slow enough that the view never flickers, alive enough to
// follow a regime change.
const basisDecayShift = 8

// foldBases advances the stable size/trade references: jump to any new
// visible max, otherwise decay geometrically. Floored at 1.
func (mk *market) foldBases() {
	var maxLevel int64
	for _, l := range mk.book.Bids {
		if l.Qty > maxLevel {
			maxLevel = l.Qty
		}
	}
	for _, l := range mk.book.Asks {
		if l.Qty > maxLevel {
			maxLevel = l.Qty
		}
	}
	var maxTrade int64
	for _, t := range mk.pending {
		if t.Qty > maxTrade {
			maxTrade = t.Qty
		}
	}
	mk.sizeBasis = foldBasis(mk.sizeBasis, maxLevel)
	mk.tradeBasis = foldBasis(mk.tradeBasis, maxTrade)
}

// foldBasis is one basis step: rise instantly to observed, else decay.
func foldBasis(basis, observed int64) int64 {
	if observed > basis {
		return observed
	}
	basis -= basis >> basisDecayShift
	if basis < 1 {
		basis = 1
	}
	return basis
}

// resizeHeat refits every market's heatmap grid to the terminal. Rows are
// price-space, so history SURVIVES a resize (each live ring trims to the new
// cap; far tiers rebuild only when their count changes). Too small to render
// zeroes the width, which the view reports.
func (m *Model) resizeHeat() {
	w, rows := streamDims(m.width, m.height)
	if w < 8 || rows < 3 {
		m.heatW = 0
		return
	}
	far := clamp((rows-1)/3, 0, maxFarRows)
	for _, mk := range m.mkts {
		mk.heat.Configure(rows-1-far, far)
	}
	m.heatW = w
}

// streamDims derives the heatmap's column count and total body-row count
// (far + live + now) from the terminal size. Horizontal budget: news rail (1)
// + heat columns + time gutter + trade-tape rail; vertical: header (2: title
// + mode line) + body + ruler (1) + footer (5). Width is forced even so the
// mid splits the axis cleanly.
func streamDims(width, height int) (int, int) {
	w := width - 1 - gutterWidth - tapeRailWidth
	if w%2 != 0 {
		w--
	}
	rows := height - (2 + 1 + 5)
	if rows > 72 {
		rows = 72
	}
	return w, rows
}

// foldRtt records a measured round-trip from a private event. A negative RttNs
// (RttUnknown) is never measured — it must not fabricate a latency figure.
func (m *Model) foldRtt(rttNs int64) {
	if rttNs < 0 {
		return
	}
	m.latWindow.Add(rttNs)
	m.lastLat = &book.Sample{
		TotalNs:    rttNs,
		NetNs:      book.NsUnknown,
		InternalNs: book.NsUnknown,
		EngineNs:   book.NsUnknown,
	}
}

// foldMdFrame records the client-measured age of an inbound marketdata frame
// and marks its arrival for staleness tracking. Age is wall-clock now minus
// the frame's server ts_ns (Unix epoch nanoseconds — rsx-types::time_utils,
// SystemTime::now().duration_since(UNIX_EPOCH)). tsNs == 0 means the frame
// carries no real timestamp (the offline demo script doesn't stamp one) —
// that's not measurable, so it's left as book.NsUnknown rather than showing
// a fabricated multi-decade age. Arrival time (for staleness) is recorded
// either way, since it just answers "did a frame land."
func (m *Model) foldMdFrame(tsNs uint64) {
	m.lastMdAt = time.Now()
	if tsNs == 0 {
		m.lastMdAgeNs = book.NsUnknown
		return
	}
	age := time.Now().UnixNano() - int64(tsNs)
	if age < 0 {
		age = 0 // clock skew guard — never show a negative age
	}
	m.lastMdAgeNs = age
	m.mdAgeWindow.Add(age)
}

// recenterLadder keeps the static price ladder's centre stable: it moves only
// when the mid drifts outside a small band, so the price axis doesn't
// reshuffle on every tick — the classic DOM anti-pattern (TT/Sierra keep the
// ladder stationary and recenter on demand).
func (m *Model) recenterLadder() {
	mid, ok := m.book.Mid()
	if !ok {
		return
	}
	const band = 6
	d := mid - m.ladderCenter
	if d < 0 {
		d = -d
	}
	if m.ladderCenter == 0 || d > band {
		m.ladderCenter = mid
	}
}

// removeOrder drops any open order with a matching oid (absent = no-op). It
// allocates a fresh slice so it never mutates a retained earlier model's
// backing array.
func (m *Model) removeOrder(oid uint64) {
	var out []OpenOrder
	for _, o := range m.openOrders {
		if o.Oid != oid {
			out = append(out, o)
		}
	}
	m.openOrders = out
}

// handleMouse maps a left-click on a ladder row to that row's price (the mouse
// analog of j/k / +/-). Only a left button-press inside the book column counts;
// motion, other buttons, and clicks outside the ladder are ignored. It sets the
// price and focuses it — it never submits (click-to-trade would bypass the
// two-enter confirm), and it clears a stale preview since the form changed.
func (m Model) handleMouse(e tea.MouseMsg) (tea.Model, tea.Cmd) {
	if e.Action != tea.MouseActionPress || e.Button != tea.MouseButtonLeft {
		return m, nil
	}
	if e.X > bookWidth+1 { // outside the book column (border + content)
		return m, nil
	}
	px, ok := m.priceAtY(e.Y)
	if !ok {
		return m, nil
	}
	m.pxBuf = m.fmtPx(px)
	m.focus = FocusPx
	m.pendingConfirm = nil
	m.status = fmt.Sprintf("price %s (click)", m.fmtPx(px))
	return m, nil
}

// handleStreamMouse maps a left-click on the heatmap to the price cursor via
// the inverse fisheye (the mouse analog of h/l/j/k). It only moves the
// cursor — firing stays on the keyboard (f / shift+1-5), so a stray click
// can never trade. Book view only.
func (m Model) handleStreamMouse(e tea.MouseMsg) (tea.Model, tea.Cmd) {
	if e.Action != tea.MouseActionPress || e.Button != tea.MouseButtonLeft {
		return m, nil
	}
	if m.screen != screenBook || m.heatW == 0 {
		return m, nil
	}
	col := e.X - 1 // news rail occupies column 0
	if col < 0 || col >= m.heatW {
		return m, nil
	}
	mk := m.mkt()
	px := book.FisheyePx(col, mk.heat.Anchor(), mk.heat.Tick(), m.heatW)
	if px <= 0 {
		return m, nil
	}
	mk.cursorPx = px
	m.status = fmt.Sprintf("cursor %s (click)", m.fmtPx(px))
	return m, nil
}

// handleKey applies one key. Digits / backspace / tab / b / s / t / r / p edit
// the form (and always invalidate a stale confirm preview); the rest are
// commands. Mirrors rsx-tui/src/input.rs plus specs/2/55-terminal.md.
func (m Model) handleKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	key := k.String()
	// The help overlay is modal: any key dismisses it (q still quits), so it
	// never traps the trader.
	if m.showHelp {
		if key == "q" || key == "ctrl+c" {
			return m, tea.Quit
		}
		m.showHelp = false
		return m, nil
	}
	switch key {
	case "q", "ctrl+c":
		return m, tea.Quit
	case "?":
		m.showHelp = true
		return m, nil
	case "esc":
		if m.pendingConfirm != nil {
			m.pendingConfirm = nil
			m.status = "order not sent"
			return m, nil
		}
		return m, tea.Quit
	case "enter":
		return m.handleEnter()
	case "c":
		return m.handleCancel()
	case "X":
		return m.handleCancelAll()
	case "x":
		return m.handleFlatten()
	case "R":
		return m.handleReverse()
	case "m":
		return m.handleMarket()
	case "up":
		m.orderSel = m.clampSel(m.orderSel - 1)
		return m, nil
	case "down":
		m.orderSel = m.clampSel(m.orderSel + 1)
		return m, nil
	case "f3":
		m.showTrace = !m.showTrace
		return m, nil
	case "f2":
		m.armed = !m.armed
		m.pendingConfirm = nil
		if m.armed {
			m.status = "ARMED — orders fire on one enter (no confirm)"
		} else {
			m.status = "confirm on — two-enter preview restored"
		}
		return m, nil
	}

	// Editing keys. Each also clears any pending confirm — its preview would
	// be stale once the form changes.
	edited := true
	switch {
	case len(key) == 1 && key[0] >= '0' && key[0] <= '9':
		m.appendDigit(rune(key[0]))
	case key == ".":
		m.appendDot()
	case key == "backspace":
		m.backspace()
	case key == "tab":
		if m.focus == FocusPx {
			m.focus = FocusQty
		} else {
			m.focus = FocusPx
		}
	case key == "b":
		m.side = wire.Buy
	case key == "s":
		m.side = wire.Sell
	case key == "t":
		m.tif = m.tif.Next()
	case key == "r":
		m.reduceOnly = !m.reduceOnly
	case key == "p":
		m.postOnly = !m.postOnly
	case key == "+" || key == "=":
		m.stepPx(1)
	case key == "-":
		m.stepPx(-1)
	case key == "j":
		m.joinBid()
	case key == "k":
		m.joinAsk()
	default:
		edited = false
	}
	if edited {
		m.pendingConfirm = nil
	}
	return m, nil
}

// tick is the configured price increment, floored at 1 so the nudge keys always
// move by something even when unset.
func (m Model) tick() int64 {
	if m.cfg.Tick > 0 {
		return m.cfg.Tick
	}
	return 1
}

// currentPx is the raw price the buffer holds (decimal input reconstructed to
// raw), or a seed (mid rounded down to the tick, else one tick) when it's
// empty/unparseable — so `+`/`-` work before a price is typed.
func (m Model) currentPx() int64 {
	if px, ok := parseRaw(m.pxBuf, m.cfg.PriceDec); ok && px > 0 {
		return px
	}
	if mid, ok := m.book.Mid(); ok && mid > 0 {
		t := m.tick()
		return (mid / t) * t
	}
	return m.tick()
}

// stepPx nudges the price buffer by n ticks (n may be negative), flooring at one
// tick so it never reaches zero or negative, and focuses the price field. The
// buffer is written back as the human decimal the trader reads.
func (m *Model) stepPx(n int64) {
	px := m.currentPx() + n*m.tick()
	if px < m.tick() {
		px = m.tick()
	}
	m.pxBuf = m.fmtPx(px)
	m.focus = FocusPx
}

// joinBid / joinAsk set the price buffer to the current best bid / ask (as the
// human decimal) — the keyboard analog of clicking that level to rest at the
// touch.
func (m *Model) joinBid() {
	b, ok := m.book.BestBid()
	if !ok {
		m.status = "no bid to join"
		return
	}
	m.pxBuf = m.fmtPx(b.Px)
	m.focus = FocusPx
}

func (m *Model) joinAsk() {
	a, ok := m.book.BestAsk()
	if !ok {
		m.status = "no ask to join"
		return
	}
	m.pxBuf = m.fmtPx(a.Px)
	m.focus = FocusPx
}

// appendDot adds a single decimal point to the focused buffer — ignored if one
// is already there, the field has no fractional precision, or the buffer is
// full. A leading dot is expanded to "0." so ".5" reads as "0.5".
func (m *Model) appendDot() {
	buf, dec := &m.pxBuf, m.cfg.PriceDec
	if m.focus == FocusQty {
		buf, dec = &m.qtyBuf, m.cfg.QtyDec
	}
	if dec <= 0 || strings.ContainsRune(*buf, '.') || len(*buf) >= digitCap {
		return
	}
	if *buf == "" {
		*buf = "0"
	}
	*buf += "."
}

// appendDigit adds r to the focused buffer, capping length so it always parses.
func (m *Model) appendDigit(r rune) {
	if m.focus == FocusPx {
		if len(m.pxBuf) < digitCap {
			m.pxBuf += string(r)
		}
		return
	}
	if len(m.qtyBuf) < digitCap {
		m.qtyBuf += string(r)
	}
}

// backspace pops the last digit off the focused buffer.
func (m *Model) backspace() {
	if m.focus == FocusPx {
		if len(m.pxBuf) > 0 {
			m.pxBuf = m.pxBuf[:len(m.pxBuf)-1]
		}
		return
	}
	if len(m.qtyBuf) > 0 {
		m.qtyBuf = m.qtyBuf[:len(m.qtyBuf)-1]
	}
}

// handleEnter is the two-step confirm gate: the first enter builds a preview
// and returns without submitting; only a second, separate enter — with
// pendingConfirm already set from the first call — reaches Submit. Since
// Bubble Tea delivers one KeyMsg per Update call, a rapid double-enter is
// still two distinct calls through this gate: the preview always renders
// (viewConfirm, driven by pendingConfirm) before the send branch can run.
func (m Model) handleEnter() (tea.Model, tea.Cmd) {
	if m.pendingConfirm == nil {
		o, ok := m.buildOrder()
		if !ok {
			m.status = "incomplete order (need price & qty)"
			return m, nil
		}
		return m.arm(o)
	}

	o := *m.pendingConfirm
	if err := m.cfg.Sub.Submit(o); err != nil {
		// Keep the preview set so a retry-enter resubmits.
		m.status = "submit failed: " + err.Error()
		return m, nil
	}
	m.status = m.sentStatus(o)
	m.clearForm()
	return m, nil
}

// clearForm resets the order-entry fields after a send, keeping side and tif
// for the next order (a trader usually works one side at a time).
func (m *Model) clearForm() {
	m.pxBuf = ""
	m.qtyBuf = ""
	m.focus = FocusPx
	m.reduceOnly = false
	m.postOnly = false
	m.pendingConfirm = nil
}

// buildOrder parses the form (human-decimal px/qty) into a raw-i64 OrderReq, or
// false if either buffer is empty / unparseable / non-positive.
func (m Model) buildOrder() (wire.OrderReq, bool) {
	px, okPx := parseRaw(m.pxBuf, m.cfg.PriceDec)
	qty, okQty := parseRaw(m.qtyBuf, m.cfg.QtyDec)
	if !okPx || !okQty || px <= 0 || qty <= 0 {
		return wire.OrderReq{}, false
	}
	return wire.OrderReq{
		Side:       m.side,
		Px:         px,
		Qty:        qty,
		Tif:        m.tif,
		ReduceOnly: m.reduceOnly,
		PostOnly:   m.postOnly,
	}, true
}

// sentStatus renders the confirmation line for a submitted order in human
// decimals, e.g. "sent BUY 5.0000 @ 0.010001 [GTC] ro po".
func (m Model) sentStatus(o wire.OrderReq) string {
	s := fmt.Sprintf("sent %s %s @ %s [%s]", o.Side.Label(), m.fmtQty(o.Qty), m.fmtPx(o.Px), o.Tif.Label())
	if o.ReduceOnly {
		s += " ro"
	}
	if o.PostOnly {
		s += " po"
	}
	return s
}

// handleFlatten begins closing the whole derived net position: 'x' builds
// the reduce-only, marketable order that flattens it (Sell at the best bid
// to close a long, Buy at the best ask to close a short, qty = |net|) and
// routes it through the same pendingConfirm gate handleEnter's second
// press already drives — a flatten is never a single fat-fingered
// keystroke. A flat position, or a missing opposing best price (no live
// book to price against), is a no-op with a status explaining why; it
// never fabricates a price.
func (m Model) handleFlatten() (tea.Model, tea.Cmd) {
	if m.position.Flat() {
		m.status = "no position to flatten"
		return m, nil
	}
	o, ok := m.buildFlattenOrder()
	if !ok {
		m.status = "can't flatten: no live book to price against"
		return m, nil
	}
	return m.arm(o)
}

// handleMarket arms a marketable IOC at the far touch for the form's side +
// qty (Buy crosses the best ask, Sell the best bid) — take liquidity now,
// routed through the same confirm gate. Empty/bad qty or no opposing book to
// cross is a no-op with a reason; it never fabricates a price.
func (m Model) handleMarket() (tea.Model, tea.Cmd) {
	qty, ok := parseRaw(m.qtyBuf, m.cfg.QtyDec)
	if !ok || qty <= 0 {
		m.status = "market: enter a qty first"
		return m, nil
	}
	var px int64
	if m.side == wire.Buy {
		if len(m.book.Asks) == 0 {
			m.status = "market: no ask to cross"
			return m, nil
		}
		px = m.book.Asks[0].Px
	} else {
		if len(m.book.Bids) == 0 {
			m.status = "market: no bid to cross"
			return m, nil
		}
		px = m.book.Bids[0].Px
	}
	o := wire.OrderReq{Side: m.side, Px: px, Qty: qty, Tif: wire.Ioc, ReduceOnly: m.reduceOnly}
	return m.arm(o)
}

// maxOrderQty is the fat-finger hard cap (lots). An order over it is BLOCKED
// outright — a hard stop, not a dismissible soft warning (the Citi fat-finger
// lesson: soft warnings train click-through).
const maxOrderQty = 1_000_000

// arm applies the fat-finger guard, then either sets the confirm preview or —
// in ARMED (confirm-off) mode — submits straight away. Every order path (enter
// / market / flatten / reverse) goes through it, so nothing oversized can even
// reach the confirm, and ARMED uniformly skips the preview for all of them
// while the size guard still holds.
func (m Model) arm(o wire.OrderReq) (tea.Model, tea.Cmd) {
	if o.Qty > maxOrderQty {
		m.status = fmt.Sprintf("BLOCKED: qty %s exceeds max %s (fat-finger guard)", m.fmtQty(o.Qty), m.fmtQty(maxOrderQty))
		return m, nil
	}
	if m.armed {
		if err := m.cfg.Sub.Submit(o); err != nil {
			m.status = "submit failed: " + err.Error()
			return m, nil
		}
		m.status = m.sentStatus(o)
		m.clearForm()
		return m, nil
	}
	m.pendingConfirm = &o
	return m, nil
}

// handleReverse flips the net position: a marketable order of twice the net
// size on the opposite side (+N → -N), through the confirm gate. Not
// reduce-only — it deliberately crosses zero. Flat, or no book to cross, is a
// no-op with a reason.
func (m Model) handleReverse() (tea.Model, tea.Cmd) {
	net := m.position.Net
	if net == 0 {
		m.status = "no position to reverse"
		return m, nil
	}
	side, px := wire.Sell, int64(0)
	if net < 0 { // short → buy to flip long
		side = wire.Buy
		if len(m.book.Asks) == 0 {
			m.status = "reverse: no ask to cross"
			return m, nil
		}
		px = m.book.Asks[0].Px
	} else { // long → sell to flip short
		if len(m.book.Bids) == 0 {
			m.status = "reverse: no bid to cross"
			return m, nil
		}
		px = m.book.Bids[0].Px
	}
	qty := net
	if qty < 0 {
		qty = -qty
	}
	return m.arm(wire.OrderReq{Side: side, Px: px, Qty: 2 * qty, Tif: wire.Ioc})
}

// buildFlattenOrder builds the reduce-only IOC order that flattens the
// current derived position at the best opposing price: Sell at the best
// bid to close a long, Buy at the best ask to close a short. Reduce-only
// means the exchange never lets this order carry the position past flat
// (it can only shrink toward zero), so it can never flip the position.
// Returns false when that side of the book is empty.
func (m Model) buildFlattenOrder() (wire.OrderReq, bool) {
	net := m.position.Net
	if net == 0 {
		return wire.OrderReq{}, false
	}
	qty := net
	if qty < 0 {
		qty = -qty
	}
	if net > 0 {
		bid, ok := m.book.BestBid()
		if !ok {
			return wire.OrderReq{}, false
		}
		return wire.OrderReq{Side: wire.Sell, Px: bid.Px, Qty: qty, Tif: wire.Ioc, ReduceOnly: true}, true
	}
	ask, ok := m.book.BestAsk()
	if !ok {
		return wire.OrderReq{}, false
	}
	return wire.OrderReq{Side: wire.Buy, Px: ask.Px, Qty: qty, Tif: wire.Ioc, ReduceOnly: true}, true
}

// clampSel keeps an open-orders selection index in [0, len-1], or 0 when the
// list is empty, so the cursor is always valid as orders come and go.
func (m Model) clampSel(i int) int {
	n := len(m.openOrders)
	if n == 0 {
		return 0
	}
	return clamp(i, 0, n-1)
}

// handleCancel cancels the *selected* working order (up/down move the cursor,
// the panel + ladder show which one) — not a blind cancel-newest, so a trader
// can target a specific resting order.
func (m Model) handleCancel() (tea.Model, tea.Cmd) {
	if len(m.openOrders) == 0 {
		m.status = "no open order to cancel"
		return m, nil
	}
	o := m.openOrders[m.clampSel(m.orderSel)]
	if o.Cid == "" {
		m.status = "selected order has no cid yet"
		return m, nil
	}
	if err := m.cfg.Sub.Cancel(o.Cid); err != nil {
		m.status = "cancel failed: " + err.Error()
	} else {
		m.status = "cancel sent for order " + fmt.Sprint(o.Oid)
	}
	return m, nil
}

// handleCancelAll cancels every working order carrying a cid.
func (m Model) handleCancelAll() (tea.Model, tea.Cmd) {
	n := 0
	for _, o := range m.openOrders {
		if o.Cid == "" {
			continue
		}
		if err := m.cfg.Sub.Cancel(o.Cid); err != nil {
			m.status = "cancel-all failed: " + err.Error()
			return m, nil
		}
		n++
	}
	if n == 0 {
		m.status = "no open orders to cancel"
	} else {
		m.status = fmt.Sprintf("cancel-all sent (%d orders)", n)
	}
	return m, nil
}
