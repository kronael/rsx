// Package glass is the native desktop renderer for the rsx-term visual
// language: the log-time liquidity heatmap, drawn as a grid of coloured cells
// on the GPU (or in software) instead of a terminal.
//
// The design law is the movable seam. The whole picture is a pure function of
// the newest model state, expressed as a Grid of plain-data Cells; a Renderer
// turns that Grid into pixels. All GPU/native code lives behind the Renderer
// interface (package gpu), so the backend can be swapped without touching the
// model, the Grid, or the app. rsx-term stays the template — same palette, same
// "colour = meaning" rule, same honesty markers (see CLAUDE.md).
package glass

import (
	"image/color"
	"strconv"
)

// Cell is one character-cell of the screen: a background colour (the resting
// SIZE channel — the one sequential ramp), a foreground colour, and a glyph
// (the SHAPE channel — count density, trade magnitude, persistence, or text).
// It is plain data with no behaviour: the stable boundary between the model and
// any renderer. Colours are RESOLVED here (by the Grid builder, per VISUALS.md),
// so a Renderer is a dumb rasteriser and swapping backends can never change the
// palette.
type Cell struct {
	Bg    color.RGBA
	Fg    color.RGBA
	Glyph rune
}

// Grid is the whole screen as a fixed W×H character grid, row-major. It mirrors
// rsx-term's cell model (VISUALS.md); the software renderer's Grid→pixels is the
// direct analogue of rsx-term's cells→ANSI.
type Grid struct {
	W     int
	H     int
	Cells []Cell
}

// NewGrid allocates a W×H grid cleared to the page background.
func NewGrid(w, h int) *Grid {
	g := &Grid{W: w, H: h, Cells: make([]Cell, w*h)}
	g.Clear()
	return g
}

// Clear resets every cell to the page background and a blank glyph, so a grid
// can be reused across frames without reallocating.
func (g *Grid) Clear() {
	blank := Cell{Bg: Page, Fg: Text, Glyph: ' '}
	for i := range g.Cells {
		g.Cells[i] = blank
	}
}

// At returns a pointer to the cell at (x, y), or nil if out of bounds.
func (g *Grid) At(x, y int) *Cell {
	if x < 0 || y < 0 || x >= g.W || y >= g.H {
		return nil
	}
	return &g.Cells[y*g.W+x]
}

// text writes s left-to-right from (x, y) in the given colours, stopping at the
// row's right edge. bg zero means "leave the cell's background". Returns the
// column just past the written text.
func (g *Grid) text(x, y int, s string, fg, bg color.RGBA) int {
	for _, r := range s {
		c := g.At(x, y)
		if c == nil {
			break
		}
		c.Glyph = r
		c.Fg = fg
		if bg != (color.RGBA{}) {
			c.Bg = bg
		}
		x++
	}
	return x
}

// Ayam Cemani palette — verbatim from rsx-term/ui/styles.go (spec 55). Colour
// is meaning, never decoration: add a colour only for a new meaning. These are
// the subset the read-only heatmap viewer uses.
var (
	Page       = hexColor("#040806") // page background + ramp floor
	Live       = hexColor("#22f5a1") // live / long / bid / filled / positive
	Ask        = hexColor("#f87171") // short / ask / down / reject / negative
	Heading    = hexColor("#bd83ff") // heading / badge
	Text       = hexColor("#a9bcb2") // body text
	TextBright = hexColor("#e7eeea") // focused / active / bright
	Muted      = hexColor("#586b62") // labels / captions / derived (with ~)
	Degraded   = hexColor("#fbbf24") // stale / offline / degraded
)

// hexColor parses "#rrggbb" into an opaque RGBA.
func hexColor(h string) color.RGBA {
	r, g, b := hexRGB(h)
	return color.RGBA{R: uint8(r), G: uint8(g), B: uint8(b), A: 0xff}
}

// hexRGB parses "#rrggbb" into its channels (0 on a malformed value).
func hexRGB(h string) (int, int, int) {
	if len(h) != 7 || h[0] != '#' {
		return 0, 0, 0
	}
	r, _ := strconv.ParseInt(h[1:3], 16, 0)
	g, _ := strconv.ParseInt(h[3:5], 16, 0)
	b, _ := strconv.ParseInt(h[5:7], 16, 0)
	return int(r), int(g), int(b)
}

// blend linearly interpolates two colours, t in [0,1] — the size ramp's basis.
func blend(a, b color.RGBA, t float64) color.RGBA {
	if t <= 0 {
		return a
	}
	if t >= 1 {
		return b
	}
	lerp := func(x, y uint8) uint8 { return uint8(int(x) + int(float64(int(y)-int(x))*t)) }
	return color.RGBA{R: lerp(a.R, b.R), G: lerp(a.G, b.G), B: lerp(a.B, b.B), A: 0xff}
}
