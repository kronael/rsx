package glass

import (
	"fmt"
	"image/color"
)

// AccountRow is one read-only user+wallet summary row: identity+role (real,
// rsx-auth's finished /roles/users) and financial fields (the gateway's
// spec-7 /v1/api/support/*, STUB/FIXTURE until the gateway ships it — same
// contract as specs/14-contract-support-assistant.md documents). rsx-glass has no HTTP
// client of its own yet; ComposeAccounts takes already-fetched rows, so
// wiring a live poller (mirroring live/'s WS subscriber) is a follow-up, not
// a redesign of this render path.
type AccountRow struct {
	UserID        uint32
	Login         string
	Role          string
	Collateral    int64
	Equity        int64
	UnrealizedPnl int64
}

// ComposeAccounts renders the read-only accounts/wallet overview: a title, a
// column header, then one row per account — role dashed when the user has
// none, uPnL coloured Live/Ask by sign (colour = meaning, never decoration,
// per CLAUDE.md). This is a TABLE, not the heatmap: it follows the USERS screen of
// specs/08-screen-users.md, not a new invented visual. Interactive CRUD is
// specified there (US-*) and not built; this is read-only for now.
func ComposeAccounts(g *Grid, rows []AccountRow) {
	g.Clear()
	if g.W < 40 || g.H < 5 {
		return
	}
	g.text(1, 0, "RSX — accounts (read-only overview)", TextBright, color.RGBA{})
	header := fmt.Sprintf("%-6s %-14s %-8s %12s %12s %12s",
		"id", "login", "role", "collateral", "equity", "uPnL")
	g.text(1, 2, header, Muted, color.RGBA{})

	if len(rows) == 0 {
		g.text(1, 3, "no accounts (data source offline)", Degraded, color.RGBA{})
		return
	}
	for i, r := range rows {
		y := 3 + i
		if y >= g.H-1 {
			g.text(1, y, fmt.Sprintf("… %d more", len(rows)-i), Muted, color.RGBA{})
			break
		}
		role := r.Role
		if role == "" {
			role = "—"
		}
		row := fmt.Sprintf("%-6d %-14s %-8s %12d %12d ",
			r.UserID, truncateCell(r.Login, 14), role, r.Collateral, r.Equity)
		x := g.text(1, y, row, Text, color.RGBA{})

		pnlColor := Text
		switch {
		case r.UnrealizedPnl > 0:
			pnlColor = Live
		case r.UnrealizedPnl < 0:
			pnlColor = Ask
		}
		g.text(x, y, fmt.Sprintf("%+d", r.UnrealizedPnl), pnlColor, color.RGBA{})
	}
}

// truncateCell shortens s to fit n columns, marking the cut with an
// ellipsis — never silently wraps into the next cell.
func truncateCell(s string, n int) string {
	if len(s) <= n {
		return s
	}
	if n <= 1 {
		return s[:n]
	}
	return s[:n-1] + "…"
}

// FixtureAccountRows is deterministic demo data — the same role demo.go's
// Synth plays for the heatmap: something real to render without a live
// cluster, and a reproducible scene for the golden. Values match
// the fixture accounts of specs/14-contract-support-assistant.md.
func FixtureAccountRows() []AccountRow {
	return []AccountRow{
		{UserID: 1, Login: "alice", Role: "admin", Collateral: 100_000_000, Equity: 104_500_000, UnrealizedPnl: 4_500_000},
		{UserID: 2, Login: "bob", Role: "", Collateral: 25_000_000, Equity: 21_200_000, UnrealizedPnl: -3_800_000},
	}
}
