package ui

import "github.com/charmbracelet/lipgloss"

// Ayam Cemani palette — the exchange's shared colours, one place mapping
// meaning -> lipgloss RGB. Mirrors rsx-tui/src/palette.rs and the dashboard's
// Tailwind retune (rsx-playground/pages.py); hexes verbatim, see
// specs/2/55-terminal.md. Colour is meaning, never decoration — add a const
// only for a new *meaning*, never to "look nice".

// ColorLive is live / long / bid / filled — the neon beetle-green.
const ColorLive = lipgloss.Color("#22f5a1")

// ColorBid is the bid side of the book (same green as live).
const ColorBid = ColorLive

// ColorAsk is short / ask / down / reject.
const ColorAsk = lipgloss.Color("#f87171")

// ColorHeading is section heading / badge / the ⚡ speed motif — the violet sheen.
const ColorHeading = lipgloss.Color("#bd83ff")

// ColorAccent is info / secondary accent (the lighter violet).
const ColorAccent = lipgloss.Color("#a992ff")

// ColorRing is the overlay ring (the darker violet), e.g. the confirm / trace border.
const ColorRing = lipgloss.Color("#7c3aed")

// ColorText is body text.
const ColorText = lipgloss.Color("#a9bcb2")

// ColorTextBright is bright text — focused field, active status line.
const ColorTextBright = lipgloss.Color("#e7eeea")

// ColorMuted is muted — labels, captions, help, dim / secondary.
const ColorMuted = lipgloss.Color("#586b62")

// ColorDegraded is degraded / stale / offline — the warning amber.
const ColorDegraded = lipgloss.Color("#fbbf24")

// ColorPanelBg is the panel background.
const ColorPanelBg = lipgloss.Color("#0d1712")

// ColorPageBg is the page background (darkest slate).
const ColorPageBg = lipgloss.Color("#040806")

// ColorBorder is the panel border.
const ColorBorder = lipgloss.Color("#16211b")

// Reusable styles view.go builds on. Colour carries meaning; these are just
// the common foreground pairings plus the standard bordered panel.
var (
	// StyleMuted renders labels, captions, help, and dim secondary text.
	StyleMuted = lipgloss.NewStyle().Foreground(ColorMuted)
	// StyleText renders body text.
	StyleText = lipgloss.NewStyle().Foreground(ColorText)
	// StyleTextBright renders the focused field and the active status line.
	StyleTextBright = lipgloss.NewStyle().Foreground(ColorTextBright)
	// StyleLive renders live / long / bid / positive figures.
	StyleLive = lipgloss.NewStyle().Foreground(ColorLive)
	// StyleAsk renders short / ask / down / negative figures.
	StyleAsk = lipgloss.NewStyle().Foreground(ColorAsk)
	// StyleHeading renders headings, the badge, and the ⚡ speed motif.
	StyleHeading = lipgloss.NewStyle().Foreground(ColorHeading)
	// StyleDegraded renders degraded / stale / offline states.
	StyleDegraded = lipgloss.NewStyle().Foreground(ColorDegraded)
	// StyleDerived renders client-computed values (mark, uPnL) — dim +
	// italic, paired with a "~" prefix, so they never read as
	// exchange-authoritative data. Also used for a legitimately-not-yet-real
	// latency leg (the "·· pending" marker): same rule, it's not
	// exchange-authoritative data yet either.
	StyleDerived = lipgloss.NewStyle().Foreground(ColorMuted).Italic(true)
	// StyleAccent renders the secondary-accent violet — the sparkline, the
	// one detail meant to draw the eye without competing with StyleHeading.
	StyleAccent = lipgloss.NewStyle().Foreground(ColorAccent)

	// PanelStyle is the standard bordered panel: a normal border in the muted
	// border colour. Panels carry their title as their first content line.
	PanelStyle = lipgloss.NewStyle().
			Border(lipgloss.NormalBorder()).
			BorderForeground(ColorBorder)

	// RingPanelStyle borders an overlay/preview block in the violet ring —
	// used by the confirm preview and the trace HUD.
	RingPanelStyle = lipgloss.NewStyle().
			Border(lipgloss.NormalBorder()).
			BorderForeground(ColorRing)
)
