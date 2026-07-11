// Package assistant wires the terminal's LLM pane to a locally deployed
// arizuko agent: prompt.go serializes a handoff into the plain-text prompt the
// agent receives, client.go streams the SSE reply back. It is UI-agnostic — no
// terminal types (lipgloss / bubbletea / the ui.Model) leak in, so the handoff
// and its tests outlive the pane and a future frontend reuses them.
package assistant

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"rsx-term/news"
	"rsx-term/wire"
)

// topLevels caps how many frozen levels per side land in the prompt — the
// touch and a little depth, matching the pane's own snapshot line.
const topLevels = 3

// Snapshot is the trader's client-folded account state at handoff — the
// position / open orders / session fills the terminal already derives from its
// own fill stream (ui/update.go). It is plain data (raw i64 + the symbol's
// display precision) so Render stays a pure function; the flags mark unknowns
// that Render dashes rather than fabricates.
type Snapshot struct {
	// PriceDec / QtyDec convert the raw i64 px / qty in this snapshot AND in the
	// handoff context to human decimals (raw / 10^dec).
	PriceDec int
	QtyDec   int
	// Net is the signed position in lots (raw qty units); 0 = flat.
	Net int64
	// Entry is the average entry price (raw px); valid only when HasEntry.
	Entry    int64
	HasEntry bool
	// Upnl is the unrealized P&L at the frozen mid, in raw price×qty scale
	// (Net*mark - Cost); valid only when HasUpnl.
	Upnl    int64
	HasUpnl bool
	// Fills is this session's fill count.
	Fills int
	// Orders are this session's resting orders on the handed-off symbol.
	Orders []SnapshotOrder
}

// SnapshotOrder is one resting order's side and raw px / qty.
type SnapshotOrder struct {
	Side wire.Side
	Px   int64
	Qty  int64
}

// Render serializes the handoff into the [RSX CONTEXT] / [TRADER STATE] prompt
// the agent receives: origin, market, headline-or-note, the frozen mid + top
// levels, then the trader's folded position / orders / fills. Pure and
// unknown-honest — a missing mid, entry, uPnL, or empty side renders as a dash
// (or "flat" / "none"), never a fabricated figure.
func Render(ctx news.AssistantContext, snap Snapshot) string {
	var b strings.Builder
	b.WriteString("[RSX CONTEXT]\n")
	b.WriteString("origin: " + ctx.Origin.Label() + "\n")
	ts := time.Unix(0, ctx.TsNs).UTC().Format("2006-01-02 15:04:05 UTC")
	b.WriteString("market: " + ctx.Venue + " · " + ctx.Symbol + "  at " + ts + "\n")
	if ctx.Origin == news.OriginNews && ctx.Headline != nil {
		h := *ctx.Headline
		b.WriteString(fmt.Sprintf("headline: %s  (%s · severity %d)\n", h.Text, h.Source, h.Tier))
	} else if ctx.Note != "" {
		b.WriteString("note: " + ctx.Note + "\n")
	}
	mid := "—"
	if ctx.MidPx > 0 {
		mid = decimal(ctx.MidPx, snap.PriceDec)
	}
	b.WriteString("mid: " + mid + "\n")
	b.WriteString("asks: " + levelText(ctx.Asks, snap.PriceDec, snap.QtyDec) + "\n")
	b.WriteString("bids: " + levelText(ctx.Bids, snap.PriceDec, snap.QtyDec) + "\n")

	b.WriteString("\n[TRADER STATE]\n")
	b.WriteString("position: " + positionText(snap) + "\n")
	b.WriteString("open orders: " + ordersText(snap) + "\n")
	b.WriteString(fmt.Sprintf("fills this session: %d\n", snap.Fills))
	return b.String()
}

// positionText renders the net position with entry and uPnL, dashing entry /
// uPnL when the computation is unavailable and reading "flat" when closed.
func positionText(snap Snapshot) string {
	if snap.Net == 0 {
		return "flat"
	}
	net := decimal(snap.Net, snap.QtyDec)
	if snap.Net > 0 {
		net = "+" + net
	}
	entry := "—"
	if snap.HasEntry {
		entry = decimal(snap.Entry, snap.PriceDec)
	}
	upnl := "—"
	if snap.HasUpnl {
		upnl = signedNotional(snap.Upnl, snap.PriceDec, snap.QtyDec)
	}
	return fmt.Sprintf("%s  entry %s  uPnL %s", net, entry, upnl)
}

// ordersText renders this session's resting orders, or "none".
func ordersText(snap Snapshot) string {
	if len(snap.Orders) == 0 {
		return "none"
	}
	parts := make([]string, 0, len(snap.Orders))
	for _, o := range snap.Orders {
		parts = append(parts, fmt.Sprintf("%s %s×%s", o.Side.Label(), decimal(o.Px, snap.PriceDec), decimal(o.Qty, snap.QtyDec)))
	}
	return strings.Join(parts, ", ")
}

// levelText renders up to topLevels frozen levels of one side as px×qty, or a
// dash when the side is empty.
func levelText(levels []wire.Level, priceDec, qtyDec int) string {
	if len(levels) == 0 {
		return "—"
	}
	n := len(levels)
	if n > topLevels {
		n = topLevels
	}
	parts := make([]string, 0, n)
	for _, l := range levels[:n] {
		parts = append(parts, decimal(l.Px, priceDec)+"×"+decimal(l.Qty, qtyDec))
	}
	return strings.Join(parts, "  ")
}

// signedNotional renders a raw price×qty-scale money figure (uPnL) at the
// quote's precision: divide out the qty scale to land at price scale, format at
// priceDec, and mark a positive figure with a leading +.
func signedNotional(raw int64, priceDec, qtyDec int) string {
	v := raw / pow10(qtyDec)
	s := decimal(v, priceDec)
	if v > 0 {
		s = "+" + s
	}
	return s
}

// decimal renders a raw fixed-point i64 as a human decimal with dec places
// (raw / 10^dec) — the same display conversion as ui.fmtDec, kept local so the
// prompt's number format is self-contained and testable without the ui layer.
func decimal(raw int64, dec int) string {
	if dec <= 0 {
		return strconv.FormatInt(raw, 10)
	}
	neg := raw < 0
	if neg {
		raw = -raw
	}
	scale := pow10(dec)
	s := fmt.Sprintf("%d.%0*d", raw/scale, dec, raw%scale)
	if neg {
		s = "-" + s
	}
	return s
}

// pow10 is 10^n as i64 (n small, a display-precision scale).
func pow10(n int) int64 {
	out := int64(1)
	for i := 0; i < n; i++ {
		out *= 10
	}
	return out
}
