package ui

import (
	"fmt"
	"sort"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"rsx-term/book"
	"rsx-term/news"
	"rsx-term/wire"
)

// The NEWS view: market context at a glance — a finviz-style sector map of
// the breadth venue (tiles coloured by move on a diverging scale), a
// cross-symbol co-movement overview (each symbol's ~6s together-ness with a
// BTC/ETH reference — breadth, NOT a trading grid, and NOT a lead-lag/causal
// read), then the searchable Tree of Alpha feed. Severity is the rail glyph
// language (newsMarker) reused per headline. From here one keypress goes DOWN
// an altitude: a symbol's letter jumps into its BOOK view, enter hands the
// selected headline (with the linked market's frozen book) to the ASSISTANT
// view. No TUI treemap prior art exists — a labelled grid of uniform tiles
// is the deliberate simple form.
//
// The LLM view: the assistant pane. The MODEL is a placeholder; the CONTEXT
// HANDOFF is real (news.AssistantContext, packaged on enter in the news
// view) — the pane shows exactly what a wired model would receive.

// moveTiers grade a |basis-point| move onto the diverging tile scale.
var moveTiers = []int64{10, 50, 200, 800}

// tileWidth is one sector-map tile: " BTC   +0.32% ".
const tileWidth = 15

// viewNews renders the news screen as a fixed grid: header + mode line,
// sector map, then the feed (selection ▸, search /), status + hint.
func (m Model) viewNews() string {
	lines := make([]string, 0, m.height)
	lines = append(lines, m.streamHeader(), m.modeLine())

	body := m.height - 4 // header(2) + status + hint
	tiles := m.sectorMapLines()
	maxTiles := body / 2
	if len(tiles) > maxTiles {
		tiles = tiles[:maxTiles]
	}
	lines = append(lines, tiles...)

	comove := m.coMoveLines()
	lines = append(lines, comove...)

	feedRows := body - len(tiles) - len(comove) - 1
	if feedRows < 0 {
		feedRows = 0
	}
	lines = append(lines, m.feedHeader())
	lines = append(lines, m.feedLines(feedRows)...)

	for len(lines) < m.height-2 {
		lines = append(lines, "")
	}
	lines = append(lines, StyleTextBright.Render(" "+m.status))
	lines = append(lines, m.hintLine())
	return strings.Join(lines, "\n")
}

// sectorMapLines renders the breadth venue's instruments as one line of
// move-coloured tiles per sector, sector-labelled, alphabetical.
func (m Model) sectorMapLines() []string {
	venue, ok := m.venueByName(m.watchVenue())
	if !ok {
		return nil
	}
	bySector := map[string][]Instrument{}
	for _, ins := range venue.Instruments {
		sector := ins.Sector
		if sector == "" {
			sector = "perps"
		}
		bySector[sector] = append(bySector[sector], ins)
	}
	sectors := make([]string, 0, len(bySector))
	for s := range bySector {
		sectors = append(sectors, s)
	}
	sort.Strings(sectors)

	perLine := (m.width - 9) / tileWidth
	if perLine < 1 {
		perLine = 1
	}
	var out []string
	for _, sector := range sectors {
		var sb strings.Builder
		sb.WriteString(StyleMuted.Render(fmt.Sprintf(" %-7s ", sector)))
		for i, ins := range bySector[sector] {
			if i >= perLine {
				sb.WriteString(StyleMuted.Render("…"))
				break
			}
			sb.WriteString(m.tile(ins))
		}
		out = append(out, sb.String())
	}
	return out
}

// tile renders one sector-map cell: symbol + move vs the session reference,
// on the diverging red↔green scale (theme-aware: the colorblind theme turns
// it blue↔orange). No mid yet reads neutral, never fabricated.
func (m Model) tile(ins Instrument) string {
	mk := m.marketFor(m.watchVenue(), ins.ID)
	label := fmt.Sprintf(" %-6s", tileName(ins.Name))
	bp, ok := mk.moveBp()
	if !ok {
		return StyleMuted.Render(label + fmt.Sprintf("%7s ", "—"))
	}
	sign := ""
	if bp > 0 {
		sign = "+"
	}
	text := label + fmt.Sprintf("%7s ", fmt.Sprintf("%s%d.%02d%%", sign, bp/100, abs64(bp%100)))
	hue := ColorBid
	if bp < 0 {
		hue = ColorAsk
	}
	t := float64(moveTier(abs64(bp))) / float64(len(moveTiers))
	bg := blendHex(string(ColorPageBg), string(hue), 0.15+0.85*t*0.7)
	return lipgloss.NewStyle().Foreground(ColorTextBright).Background(lipgloss.Color(bg)).Render(text)
}

// tileName compresses an instrument name for a tile ("PENGU-PERP" → "PENGU").
func tileName(name string) string {
	if i := strings.IndexByte(name, '-'); i > 0 {
		return name[:i]
	}
	return name
}

// moveTier grades an absolute bp move 0..len(moveTiers).
func moveTier(bp int64) int {
	tier := 0
	for _, threshold := range moveTiers {
		if bp >= threshold {
			tier++
		}
	}
	return tier
}

// coMoveMinBins is the fewest shared mid-return samples before a co-movement
// read is honest — under it the overview shows "gathering…" instead of a
// figure built on two or three ticks.
const coMoveMinBins = 5

// coMove is the CONTEMPORANEOUS directional co-movement of a symbol's mid with
// the reference's over their overlapping recent sparks (~6s of 100ms bins):
// the sign-agreement of per-bin mid returns, in [-1, +1] — +1 moves together,
// 0 independent, -1 inverse. This is a together-ness read, NOT a lead-lag /
// causal one: 100ms repeated-mid samples with WS jitter can't support a
// reliable lead-lag, so none is computed (deliberately). ok=false when the two
// rings share too few bins.
func coMove(sym, ref []int64) (float64, bool) {
	n := len(sym)
	if len(ref) < n {
		n = len(ref)
	}
	if n < coMoveMinBins {
		return 0, false
	}
	sym = sym[len(sym)-n:]
	ref = ref[len(ref)-n:]
	agree, disagree := 0, 0
	for i := 1; i < n; i++ {
		ds := sign64(sym[i] - sym[i-1])
		dr := sign64(ref[i] - ref[i-1])
		if ds == 0 || dr == 0 {
			continue // a flat bin says nothing about direction
		}
		if ds == dr {
			agree++
		} else {
			disagree++
		}
	}
	total := agree + disagree
	if total == 0 {
		return 0, false
	}
	return float64(agree-disagree) / float64(total), true
}

// coMoveRef picks the venue's co-movement reference instrument: BTC, else ETH,
// else the first instrument — the anchor every symbol's together-ness reads
// against.
func (m Model) coMoveRef(venue string) (Instrument, bool) {
	v, ok := m.venueByName(venue)
	if !ok || len(v.Instruments) == 0 {
		return Instrument{}, false
	}
	for _, want := range []string{"BTC", "ETH"} {
		for _, ins := range v.Instruments {
			if strings.HasPrefix(strings.ToUpper(tileName(ins.Name)), want) {
				return ins, true
			}
		}
	}
	return v.Instruments[0], true
}

// coMoveGlyph renders a co-movement value as a together-ness cell: the shape
// carries coupling (≡/= together, · independent, ≠ inverse) and the hue the
// sign (live = moves with the reference, ask = moves against it).
func coMoveGlyph(co float64) string {
	switch {
	case co >= 0.6:
		return StyleLive.Bold(true).Render("≡")
	case co >= 0.25:
		return StyleLive.Render("=")
	case co <= -0.6:
		return StyleAsk.Bold(true).Render("≠")
	case co <= -0.25:
		return StyleAsk.Render("≠")
	default:
		return StyleMuted.Render("·")
	}
}

// coMoveLines renders the cross-symbol co-movement overview: each breadth
// symbol's ~6s directional together-ness with the reference (BTC/ETH),
// most-coupled first. A breadth read, NOT a trading grid — no prices, no
// entry, and no lead-lag. Empty until enough shared bins accumulate.
func (m Model) coMoveLines() []string {
	venue := m.watchVenue()
	ref, ok := m.coMoveRef(venue)
	if !ok {
		return nil
	}
	v, _ := m.venueByName(venue)
	refSparks := m.marketFor(venue, ref.ID).sparks
	type coRow struct {
		name string
		co   float64
	}
	var rows []coRow
	for _, ins := range v.Instruments {
		if ins.ID == ref.ID {
			continue
		}
		co, ok := coMove(m.marketFor(venue, ins.ID).sparks, refSparks)
		if !ok {
			continue
		}
		rows = append(rows, coRow{tileName(ins.Name), co})
	}
	prefix := StyleMuted.Render(fmt.Sprintf(" co-move vs %s ~6s  ", tileName(ref.Name)))
	if len(rows) == 0 {
		return []string{prefix + StyleMuted.Render("gathering…")}
	}
	sort.Slice(rows, func(i, j int) bool { return rows[i].co > rows[j].co })
	perLine := (m.width - 24) / 8
	if perLine < 1 {
		perLine = 1
	}
	var sb strings.Builder
	sb.WriteString(prefix)
	for i, r := range rows {
		if i >= perLine {
			sb.WriteString(StyleMuted.Render("…"))
			break
		}
		sb.WriteString(coMoveGlyph(r.co) + StyleText.Render(fmt.Sprintf(" %-6s", r.name)))
	}
	return []string{sb.String()}
}

// feedHeader is the divider over the headline feed, carrying the live search
// buffer while searching.
func (m Model) feedHeader() string {
	title := " ── news feed "
	if !m.news.Enabled() {
		return StyleMuted.Render(title + "── feed off — set RSX_TERM_NEWS=1 ──")
	}
	if m.newsSearch || m.newsQuery != "" {
		q := m.newsQuery
		if m.newsSearch {
			q += "_"
		}
		return StyleMuted.Render(title+"── search: ") + StyleTextBright.Bold(true).Render(q) + StyleMuted.Render(" ──")
	}
	return StyleMuted.Render(title + "── / search · j/k select · enter → assistant ──")
}

// feedLines renders up to n headlines (filtered by the search query), newest
// first, the selection marked ▸.
func (m Model) feedLines(n int) []string {
	headlines := m.filteredNews()
	out := make([]string, 0, n)
	sel := clamp(m.newsSel, 0, maxInt(len(headlines)-1, 0))
	for i, h := range headlines {
		if i >= n {
			break
		}
		cursor := "  "
		if i == sel && len(headlines) > 0 {
			cursor = StyleAccent.Render("▸ ")
		}
		ts := time.Unix(0, h.TsNs).Format("15:04:05")
		text := h.Text
		if len(h.Symbols) > 0 {
			text += " [" + strings.Join(h.Symbols, " ") + "]"
		}
		budget := m.width - 16
		if budget > 0 && len(text) > budget {
			text = text[:budget-1] + "…"
		}
		line := cursor + StyleMuted.Render(ts+" ") + newsMarker(h.Tier) + " " + StyleText.Render(text)
		out = append(out, line)
	}
	if len(out) == 0 && m.news.Enabled() {
		out = append(out, StyleMuted.Render("   no headlines yet — the feed fills as news lands"))
	}
	return out
}

// filteredNews is the feed after the search query (case-insensitive match on
// text, source, and symbol tags).
func (m Model) filteredNews() []news.Marker {
	all := m.news.All()
	if m.newsQuery == "" {
		return all
	}
	q := strings.ToLower(m.newsQuery)
	var out []news.Marker
	for _, h := range all {
		if strings.Contains(strings.ToLower(h.Text), q) ||
			strings.Contains(strings.ToLower(h.Source), q) ||
			strings.Contains(strings.ToLower(strings.Join(h.Symbols, " ")), q) {
			out = append(out, h)
		}
	}
	return out
}

// handleNewsKey is the news view's grammar: / search (typed text filters the
// feed), j/k select, enter hands the selection to the assistant, a symbol's
// letter jumps into its book, esc backs out one layer.
func (m Model) handleNewsKey(act action, key string) (tea.Model, tea.Cmd) {
	switch act {
	case actBack:
		if m.newsQuery != "" {
			m.newsQuery = ""
			return m, nil
		}
		m.screen = screenBook
	case actSearch:
		m.newsSearch = true
	case actFeedDown:
		m.newsSel = m.clampNewsSel(m.newsSel + 1)
	case actFeedUp:
		m.newsSel = m.clampNewsSel(m.newsSel - 1)
	case actHandoff:
		return m.handoffToAssistant()
	default: // the fixed key class: symbol letters jump into their book
		if len(key) == 1 && key[0] >= 'a' && key[0] <= 'z' {
			if ins, ok := m.instrumentByCode(m.watchVenue(), key); ok {
				m.activeVenue = m.watchVenue()
				m.switchTo(ins)
				m.screen = screenBook
			}
		}
	}
	return m, nil
}

// handleNewsSearchKey types into the search buffer: printable chars append,
// backspace edits, enter keeps the filter, esc clears it.
func (m Model) handleNewsSearchKey(key string) (tea.Model, tea.Cmd) {
	switch {
	case key == "esc":
		m.newsSearch = false
		m.newsQuery = ""
	case key == "enter":
		m.newsSearch = false
	case key == "backspace":
		if len(m.newsQuery) > 0 {
			m.newsQuery = m.newsQuery[:len(m.newsQuery)-1]
		}
	case len(key) == 1 && key[0] >= ' ' && key[0] <= '~':
		if len(m.newsQuery) < 48 {
			m.newsQuery += key
			m.newsSel = 0
		}
	}
	return m, nil
}

// clampNewsSel keeps the feed selection on a real row.
func (m Model) clampNewsSel(i int) int {
	n := len(m.filteredNews())
	if n == 0 {
		return 0
	}
	return clamp(i, 0, n-1)
}

// handoffToAssistant packages the selected headline + its linked market's
// frozen book into a news.AssistantContext (the real contract a wired model
// will receive) and opens the assistant view.
func (m Model) handoffToAssistant() (tea.Model, tea.Cmd) {
	headlines := m.filteredNews()
	if len(headlines) == 0 {
		m.status = "no headline selected (feed empty)"
		return m, nil
	}
	h := headlines[clamp(m.newsSel, 0, len(headlines)-1)]
	venue := m.watchVenue()
	ins, ok := m.matchHeadlineSymbol(venue, h)
	if !ok { // unlinked headline: hand off the active book instead
		venue = m.activeVenue
		ins = m.ins()
	}
	mk := m.marketFor(venue, ins.ID)
	mid, _ := mk.book.Mid()
	ctx := news.PackageNews(venue, ins.Name, time.Now().UnixNano(), h, mk.book.Bids, mk.book.Asks, mid)
	m.assistCtx = &ctx
	m.assistIns = ins
	m.screen = screenLLM
	m.status = "context → assistant: " + ins.Name
	return m, nil
}

// freezeToAssistant hands the row under the BOOK microscope cursor to the
// assistant — a FREEZE of a row already in the heatmap ring, not a replay
// (there is no replay buffer). STEP 4 makes the handoff generic; today it
// reuses the news context shape with an honest book-freeze marker. Far rows
// are aggregate time-weighted windows, never restored books, and the label
// says so.
func (m Model) freezeToAssistant() (tea.Model, tea.Cmd) {
	mk := m.mkt()
	rows := mk.heat.Rows()
	if m.rowCursor < 0 || m.rowCursor >= len(rows) {
		m.status = "microscope: move the row cursor first (↑/↓)"
		return m, nil
	}
	row := rows[m.rowCursor]
	label := rowFreezeLabel(row)
	bids, asks := rowToLevels(row)
	ins := m.ins()
	ctx := news.PackageBookFreeze(m.activeVenue, ins.Name, time.Now().UnixNano(), label, bids, asks, mk.heat.MidPx())
	m.assistCtx = &ctx
	m.assistIns = ins
	m.screen = screenLLM
	m.status = "frozen → assistant: " + ins.Name + " (" + label + ")"
	return m, nil
}

// rowToLevels splits a heatmap row's price-space profile into wire bid/ask
// levels (Side<0 bid, >0 ask) for the assistant handoff.
func rowToLevels(row book.Row) (bids, asks []wire.Level) {
	for _, l := range row.Levels {
		lv := wire.Level{Px: l.Px, Qty: l.Size, Count: uint32(l.Count)}
		if l.Side < 0 {
			bids = append(bids, lv)
		} else {
			asks = append(asks, lv)
		}
	}
	return bids, asks
}

// rowFreezeLabel is a frozen row's honest description: a far row is an
// aggregate time-weighted window (NOT a restored book), a live row an exact
// ~100ms bin.
func rowFreezeLabel(row book.Row) string {
	if row.Span > 0 {
		return "~" + fmtSpan(row.Span) + " window (aggregate, not a restored book)"
	}
	return "exact ~100ms bin"
}

// matchHeadlineSymbol links a headline's symbol tags (BTC, WIFUSDT, …) to an
// instrument on the venue: exact name, or the tag prefixed by the
// instrument's base name (PENGU-PERP matches PENGUUSDT).
func (m Model) matchHeadlineSymbol(venue string, h news.Marker) (Instrument, bool) {
	v, ok := m.venueByName(venue)
	if !ok {
		return Instrument{}, false
	}
	for _, tag := range h.Symbols {
		for _, ins := range v.Instruments {
			base := strings.ToUpper(tileName(ins.Name))
			if tag == strings.ToUpper(ins.Name) || strings.HasPrefix(tag, base) {
				return ins, true
			}
		}
	}
	return Instrument{}, false
}

// viewLLM renders the assistant screen: the packaged handoff (real) over the
// reply pane (an honest placeholder until a model is wired).
func (m Model) viewLLM() string {
	lines := make([]string, 0, m.height)
	lines = append(lines, m.streamHeader(), m.modeLine())
	lines = append(lines, "", StyleHeading.Bold(true).Render("  ASSISTANT")+StyleMuted.Render("  (no model wired — the context below is exactly what one will receive)"))

	if m.assistCtx == nil {
		lines = append(lines, "", StyleMuted.Render("  no context yet — select a headline (news) or freeze a row (book microscope) and press enter"))
	} else {
		lines = append(lines, m.assistContextLines()...)
	}

	for len(lines) < m.height-2 {
		lines = append(lines, "")
	}
	lines = append(lines, StyleTextBright.Render(" "+m.status))
	lines = append(lines, m.hintLine())
	return strings.Join(lines, "\n")
}

// assistContextLines renders the frozen handoff: the market, an origin block
// (a news headline OR a book-freeze note), and the frozen level snapshot.
func (m Model) assistContextLines() []string {
	ctx := *m.assistCtx
	ins := m.assistIns
	ts := time.Unix(0, ctx.TsNs).Format("15:04:05")
	out := []string{
		"",
		StyleMuted.Render("  ── context handed off ─────────────────────────"),
		StyleMuted.Render("  market   ") + StyleTextBright.Render(ctx.Venue+" · "+ctx.Symbol) + StyleMuted.Render("  at "+ts),
	}
	out = append(out, m.assistOriginLines(ctx)...)
	mid := "—"
	if ctx.MidPx > 0 {
		mid = fmtDec(ctx.MidPx, ins.PriceDec)
	}
	out = append(out, StyleMuted.Render("  book     ")+StyleText.Render("mid "+mid+"  ("+fmt.Sprint(len(ctx.Bids))+" bid / "+fmt.Sprint(len(ctx.Asks))+" ask levels frozen)"))
	out = append(out, "  "+m.snapshotLine("asks", ctx.Asks, ins, StyleAsk))
	out = append(out, "  "+m.snapshotLine("bids", ctx.Bids, ins, StyleLive))
	out = append(out,
		"",
		StyleMuted.Render("  ── assistant reply ────────────────────────────"),
		StyleDerived.Render("  ~ placeholder — wiring an LLM is a follow-up; nothing here is generated"),
	)
	return out
}

// assistOriginLines renders the origin-specific block: a news headline (with
// its source + severity) or a book-freeze note (the honest row window). A book
// freeze carries NO headline, so it never fabricates a news marker.
func (m Model) assistOriginLines(ctx news.AssistantContext) []string {
	if ctx.Origin == news.OriginBookFreeze || ctx.Headline == nil {
		note := ctx.Note
		if note == "" {
			note = ctx.Origin.Label()
		}
		return []string{StyleMuted.Render("  origin   ") + StyleTextBright.Render(ctx.Origin.Label()) + StyleMuted.Render(" · "+note)}
	}
	h := *ctx.Headline
	hts := time.Unix(0, h.TsNs).Format("15:04:05")
	return []string{
		StyleMuted.Render("  headline ") + newsMarker(h.Tier) + " " + StyleText.Render(h.Text),
		StyleMuted.Render("           " + hts + " · " + h.Source + " · severity " + fmt.Sprint(h.Tier)),
	}
}

// snapshotLine renders up to three frozen levels of one side.
func (m Model) snapshotLine(label string, levels []wire.Level, ins Instrument, style lipgloss.Style) string {
	if len(levels) == 0 {
		return StyleMuted.Render(label + " —")
	}
	n := len(levels)
	if n > 3 {
		n = 3
	}
	parts := make([]string, 0, n)
	for _, l := range levels[:n] {
		parts = append(parts, fmtDec(l.Px, ins.PriceDec)+"×"+fmtDec(l.Qty, ins.QtyDec))
	}
	return StyleMuted.Render(label+" ") + style.Render(strings.Join(parts, "  "))
}

// handleLLMKey: back (esc) returns to the news view — the layer above.
func (m Model) handleLLMKey(act action) (tea.Model, tea.Cmd) {
	if act == actBack {
		m.screen = screenNews
	}
	return m, nil
}

func maxInt(a, b int) int {
	if a > b {
		return a
	}
	return b
}
