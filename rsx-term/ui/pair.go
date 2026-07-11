package ui

import (
	"fmt"
	"strings"

	"rsx-term/wire"
)

// The PAIR view: a multi-symbol list for CHASING and DIRECTIONAL trading —
// fast aggressive cross-pair execution, not depth-reading (that is the book
// view). One row per watched symbol:
//
//	[a] PENGU-PERP   0.010000  +0.32%  ▂█  ████  L+2.0
//	 │       │           │        │     │    │     └ position, in lots
//	 │       │           │        │     │    └ flow bar: trade intensity, hue = dominant aggressor
//	 │       │           │        │     └ depth state: bid|ask thickness vs its own recent past
//	 │       │           │        └ move vs the session reference (first recorded mid)
//	 │       │           └ mid
//	 └ the symbol's SELECT letter — press it to ARM the row
//
// Grammar (see handlePairKey): letter arms · digits set a lot count · b buys
// (lifts the offer) · s sells (hits the bid) · `.` flattens reduce-only ·
// [ ] switch watchlists · esc clears. Market/IOC always; sizes are LOTS
// (lotQty), never exact numbers.

// depthTiers are the depth-state thresholds vs the side's own slow basis:
// thin < 1/2 · normal · thick > 3/2.
func depthTierGlyph(depth, basis int64) rune {
	switch {
	case depth <= 0:
		return ' '
	case 2*depth < basis:
		return glyphs.microRamp[0] // thin
	case 2*depth > 3*basis:
		return glyphs.microRamp[len(glyphs.microRamp)-1] // thick
	default:
		return glyphs.microRamp[3] // normal
	}
}

// flowBarWidth is the trade-intensity bar's cell count.
const flowBarWidth = 4

// viewPair renders the pair screen as a fixed grid: header + mode line, one
// row per symbol on the active watchlist, padding, then status + legend.
func (m Model) viewPair() string {
	lines := make([]string, 0, m.height)
	lines = append(lines, m.streamHeader(), m.modeLine())
	rowBudget := m.height - 4 // header(2) + status + legend
	ids := m.lists[m.listSel].ids
	for i, id := range ids {
		if i >= rowBudget {
			break
		}
		lines = append(lines, m.pairRow(id))
	}
	for len(lines) < m.height-2 {
		lines = append(lines, "")
	}
	lines = append(lines, StyleTextBright.Render(" "+m.status))
	lines = append(lines, m.hintLine())
	return strings.Join(lines, "\n")
}

// pairLegend is the pair view's control hint.
const pairLegend = " q quit  tab view  letter arm  [count] b buy  s sell  . flatten  1-9 lots  [ ] list  r RO  esc clear  ? help "

// pairRow renders one symbol's line (see the file header for the anatomy).
// The armed row is highlighted — you must SEE what the next b/s fires at.
func (m Model) pairRow(id uint32) string {
	mk := m.marketFor(m.pairVenue(), id)
	armed := id == m.armedSym

	sel := StyleMuted.Render("[" + mk.ins.Code + "]")
	if armed {
		sel = StyleArmed.Render("[" + mk.ins.Code + "]")
	}
	name := fmt.Sprintf("%-12s", mk.ins.Name)
	nameStr := StyleText.Render(name)
	if armed {
		nameStr = StyleTextBright.Bold(true).Render(name)
	}

	mid, hasMid := mk.book.Mid()
	pxStr := StyleMuted.Render(fmt.Sprintf("%12s", "—"))
	if hasMid {
		pxStr = StyleTextBright.Render(fmt.Sprintf("%12s", fmtDec(mid, mk.ins.PriceDec)))
	}

	move := StyleMuted.Render(fmt.Sprintf("%8s", "—"))
	if bp, ok := mk.moveBp(); ok {
		move = fmtMoveBp(bp)
	}

	depth := m.pairDepthGlyphs(mk)
	flow := pairFlowBar(mk)
	pos := m.pairPos(mk)

	return " " + sel + " " + nameStr + pxStr + "  " + move + "  " + depth + "  " + flow + "  " + pos
}

// moveBp is the mid's move vs the session reference (the oldest retained
// spark), in basis points.
func (mk *market) moveBp() (int64, bool) {
	mid, ok := mk.book.Mid()
	if !ok || len(mk.sparks) == 0 || mk.sparks[0] <= 0 {
		return 0, false
	}
	return (mid - mk.sparks[0]) * 10_000 / mk.sparks[0], true
}

// fmtMoveBp renders a basis-point move as a signed percentage, hue by sign.
func fmtMoveBp(bp int64) string {
	style := StyleMuted
	if bp > 0 {
		style = StyleLive
	} else if bp < 0 {
		style = StyleAsk
	}
	sign := ""
	if bp > 0 {
		sign = "+"
	}
	return style.Render(fmt.Sprintf("%8s", fmt.Sprintf("%s%d.%02d%%", sign, bp/100, abs64(bp%100))))
}

func abs64(v int64) int64 {
	if v < 0 {
		return -v
	}
	return v
}

// pairDepthGlyphs is the two-rune depth state: bid then ask thickness, each
// vs half the market's own slow depth basis (position encodes side, as on
// the heatmap — no side hues in resting liquidity).
func (m Model) pairDepthGlyphs(mk *market) string {
	half := mk.depthBasis / 2
	if half < 1 {
		half = 1
	}
	bid := depthTierGlyph(sideDepth(mk.book.Bids), half)
	ask := depthTierGlyph(sideDepth(mk.book.Asks), half)
	return StyleText.Render(string(bid) + string(ask))
}

// pairFlowBar is the recent trade-flow read: bar length = intensity vs the
// market's own trade basis, hue = the dominant aggressor side.
func pairFlowBar(mk *market) string {
	total := mk.flowBuy + mk.flowSell
	n := logTier(total, mk.tradeBasis*8, flowBarWidth)
	bar := strings.Repeat("█", n) + strings.Repeat(" ", flowBarWidth-n)
	side := wire.Buy
	if mk.flowSell > mk.flowBuy {
		side = wire.Sell
	}
	if n == 0 {
		return StyleMuted.Render(bar)
	}
	return aggressorStyle(side).Render(bar)
}

// pairPos renders the symbol's derived position in LOTS (the pair view's
// risk unit; L long / S short / muted dash flat).
func (m Model) pairPos(mk *market) string {
	if mk.position.Flat() {
		return StyleMuted.Render("     —")
	}
	mid, ok := mk.book.Mid()
	if !ok {
		mid = mk.position.Cost / max64(abs64(mk.position.Net), 1)
	}
	lot, ok := m.lotQty(mk.ins, max64(mid, 1))
	if !ok || lot < 1 {
		lot = 1
	}
	tenths := mk.position.Net * 10 / lot
	word, style := "L", StyleLive
	if tenths < 0 {
		word, style = "S", StyleAsk
		tenths = -tenths
	}
	return style.Render(fmt.Sprintf("%s%s%d.%d", word, plusIf(mk.position.Net > 0), tenths/10, tenths%10))
}

func plusIf(b bool) string {
	if b {
		return "+"
	}
	return "-"
}

func max64(a, b int64) int64 {
	if a > b {
		return a
	}
	return b
}
