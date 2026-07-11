package ui

import (
	"fmt"
	"sort"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"rsx-term/news"
	"rsx-term/wire"
)

// The NEWS view: market context at a glance — a finviz-style sector map of
// the breadth venue (tiles coloured by move on a diverging scale) over the
// searchable Tree of Alpha feed. Severity is the rail glyph language
// (newsMarker) reused per headline. From here one keypress goes DOWN an
// altitude: a symbol's letter jumps into its BOOK view, enter hands the
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

	feedRows := body - len(tiles) - 1
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
	venue, ok := m.venueByName(m.pairVenue())
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
	mk := m.marketFor(m.pairVenue(), ins.ID)
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
			if ins, ok := m.instrumentByCode(m.pairVenue(), key); ok {
				m.activeVenue = m.pairVenue()
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
	venue := m.pairVenue()
	ins, ok := m.matchHeadlineSymbol(venue, h)
	if !ok { // unlinked headline: hand off the active book instead
		venue = m.activeVenue
		ins = m.ins()
	}
	mk := m.marketFor(venue, ins.ID)
	mid, _ := mk.book.Mid()
	ctx := news.PackageContext(venue, ins.Name, time.Now().UnixNano(), h, mk.book.Bids, mk.book.Asks, mid)
	m.assistCtx = &ctx
	m.assistIns = ins
	m.screen = screenLLM
	m.status = "context → assistant: " + ins.Name
	return m, nil
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
		lines = append(lines, "", StyleMuted.Render("  no context yet — select a headline in the news view and press enter"))
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

// assistContextLines renders the frozen handoff: headline, market, and the
// book snapshot at handoff time.
func (m Model) assistContextLines() []string {
	ctx := *m.assistCtx
	ins := m.assistIns
	ts := time.Unix(0, ctx.TsNs).Format("15:04:05")
	hts := time.Unix(0, ctx.Headline.TsNs).Format("15:04:05")
	out := []string{
		"",
		StyleMuted.Render("  ── context handed off ─────────────────────────"),
		StyleMuted.Render("  market   ") + StyleTextBright.Render(ctx.Venue+" · "+ctx.Symbol) + StyleMuted.Render("  at "+ts),
		StyleMuted.Render("  headline ") + newsMarker(ctx.Headline.Tier) + " " + StyleText.Render(ctx.Headline.Text),
		StyleMuted.Render("           " + hts + " · " + ctx.Headline.Source + " · severity " + fmt.Sprint(ctx.Headline.Tier)),
	}
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
