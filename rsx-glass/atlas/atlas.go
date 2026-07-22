// Package atlas is the ONE glyph bank both renderers share: one alpha-coverage
// mask (cellW*cellH bytes, 0..255) per rune. Because soft and gpu sample the
// SAME masks, a Cell's glyph rune is a fully resolved identity — the backend
// never re-chooses a font or re-draws a shape, so they cannot diverge.
//
// ASCII text is rasterised once from the embedded Go Mono font (x/image/font —
// a pure-Go rasteriser, so masks are byte-identical on any machine, which is
// what byte-locks the goldens). The non-ASCII heatmap glyphs (density blocks,
// live bars, persistence, trade shapes, rules, the honesty dash) are drawn as
// PRIMITIVES — exactly the terminal-cell workarounds the GPU layer is meant to
// replace (rsx-term/notes/element-inventory.md): the channel survives as a
// primitive, the character does not.
package atlas

import (
	"math"
	"sort"

	"golang.org/x/image/font"
	"golang.org/x/image/font/gofont/gomono"
	"golang.org/x/image/font/opentype"
	"golang.org/x/image/math/fixed"
)

// Atlas holds the shared masks and the fixed cell size.
type Atlas struct {
	cellW int
	cellH int
	masks map[rune][]byte
}

// New builds the atlas at a fixed cell size derived from the font metrics.
func New() (*Atlas, error) {
	f, err := opentype.Parse(gomono.TTF)
	if err != nil {
		return nil, err
	}
	const sizePt, dpi = 14, 72
	face, err := opentype.NewFace(f, &opentype.FaceOptions{Size: sizePt, DPI: dpi, Hinting: font.HintingFull})
	if err != nil {
		return nil, err
	}
	defer face.Close() //nolint:errcheck // face closed after masks are copied out

	m := face.Metrics()
	ascent := m.Ascent.Ceil()
	cellH := ascent + m.Descent.Ceil()
	adv, _ := face.GlyphAdvance('0')
	cellW := adv.Ceil()

	a := &Atlas{cellW: cellW, cellH: cellH, masks: make(map[rune][]byte)}
	for r := rune(0x20); r <= 0x7e; r++ { // printable ASCII from the font
		a.masks[r] = a.fontMask(face, ascent, r)
	}
	a.drawPrimitives()
	return a, nil
}

// CellSize returns the fixed cell dimensions in pixels.
func (a *Atlas) CellSize() (w, h int) { return a.cellW, a.cellH }

// Mask returns the coverage mask for r, or nil (a blank cell) if unknown.
func (a *Atlas) Mask(r rune) []byte {
	if r == ' ' || r == 0 {
		return nil
	}
	return a.masks[r]
}

// Glyphs returns every rune the atlas holds a mask for, sorted, so a backend
// that packs the masks into a single texture (the gpu path) enumerates the exact
// same set soft samples per-rune — the two renderers can never disagree on which
// glyphs exist. Blank/unknown runes are handled by the caller (a zero tile).
func (a *Atlas) Glyphs() []rune {
	runes := make([]rune, 0, len(a.masks))
	for r := range a.masks {
		runes = append(runes, r)
	}
	sort.Slice(runes, func(i, j int) bool { return runes[i] < runes[j] })
	return runes
}

// fontMask rasterises one font glyph into a cell-sized coverage mask.
func (a *Atlas) fontMask(face font.Face, ascent int, r rune) []byte {
	dot := fixed.P(0, ascent)
	dr, src, srcp, _, ok := face.Glyph(dot, r)
	if !ok {
		return nil
	}
	buf := make([]byte, a.cellW*a.cellH)
	for y := dr.Min.Y; y < dr.Max.Y; y++ {
		for x := dr.Min.X; x < dr.Max.X; x++ {
			if x < 0 || y < 0 || x >= a.cellW || y >= a.cellH {
				continue
			}
			_, _, _, av := src.At(srcp.X+(x-dr.Min.X), srcp.Y+(y-dr.Min.Y)).RGBA()
			buf[y*a.cellW+x] = uint8(av >> 8)
		}
	}
	return buf
}

// drawPrimitives fills in the non-ASCII glyphs as drawn shapes.
func (a *Atlas) drawPrimitives() {
	// Size ramp count-density blocks: a full-cell fill at ascending coverage
	// (measured ink 0.22/0.56/0.86/1.00 in DejaVuSansMono — the coarse count
	// channel; fine size rides on the background ramp).
	a.masks['░'] = a.fill(func(x, y int) uint8 { return 56 })  // 0.22
	a.masks['▒'] = a.fill(func(x, y int) uint8 { return 143 }) // 0.56
	a.masks['▓'] = a.fill(func(x, y int) uint8 { return 219 }) // 0.86
	a.masks['█'] = a.fill(func(x, y int) uint8 { return 255 }) // 1.00

	// NOW-row micro-bars: a real bottom-anchored histogram bar, height ∝ level.
	for i, r := range []rune{'▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'} {
		level := i + 1
		a.masks[r] = a.bar(level, 8)
	}

	// Persistence ▚: two opposite quadrants filled.
	a.masks['▚'] = a.fill(func(x, y int) uint8 {
		ul := x < a.cellW/2 && y < a.cellH/2
		lr := x >= a.cellW/2 && y >= a.cellH/2
		if ul || lr {
			return 255
		}
		return 0
	})

	// Trade layer ▵▴△▲ (buys) / ▿▾▽▼ (sells): mirrored ascending-ink shape
	// ladders — side by polarity (up = lifts, down = hits), magnitude by
	// size+fill — so flow never reads as resting liquidity and side never
	// rides on hue alone (rsx-term VISUALS.md).
	a.masks['▵'] = a.triangle(0.30, true, false)
	a.masks['▴'] = a.triangle(0.30, true, true)
	a.masks['△'] = a.triangle(0.44, true, false)
	a.masks['▲'] = a.triangle(0.44, true, true)
	a.masks['▿'] = a.triangle(0.30, false, false)
	a.masks['▾'] = a.triangle(0.30, false, true)
	a.masks['▽'] = a.triangle(0.44, false, false)
	a.masks['▼'] = a.triangle(0.44, false, true)

	// Header link dot ●.
	a.masks['●'] = a.disc(0.40)

	// Chrome: link dot (a small disc), ruler baseline, touch tick, honesty dash.
	a.masks['·'] = a.disc(0.14)
	a.masks['─'] = a.hline()
	a.masks['┼'] = a.cross()
	a.masks['—'] = a.dash()
	a.masks['−'] = a.dash()
}

func (a *Atlas) fill(cov func(x, y int) uint8) []byte {
	buf := make([]byte, a.cellW*a.cellH)
	for y := 0; y < a.cellH; y++ {
		for x := 0; x < a.cellW; x++ {
			buf[y*a.cellW+x] = cov(x, y)
		}
	}
	return buf
}

// bar fills the bottom level/steps of the cell (a real histogram bar).
func (a *Atlas) bar(level, steps int) []byte {
	h := level * a.cellH / steps
	if h < 1 {
		h = 1
	}
	return a.fill(func(x, y int) uint8 {
		if y >= a.cellH-h {
			return 255
		}
		return 0
	})
}

// shape rasterises a centred primitive from an inside-test, radius as a
// fraction of the cell's smaller half-extent: disc (the link dot, the honesty
// tick) and triangle (the trade-side ladder) are its only two callers.
func (a *Atlas) shape(rf float64, in func(dx, dy, r float64) bool) []byte {
	cx, cy := float64(a.cellW-1)/2, float64(a.cellH-1)/2
	r := rf * math.Min(cx, cy) * 2
	return a.fill(func(x, y int) uint8 {
		if in(float64(x)-cx, float64(y)-cy, r) {
			return 255
		}
		return 0
	})
}

func (a *Atlas) disc(rf float64) []byte {
	return a.shape(rf, func(dx, dy, r float64) bool { return dx*dx+dy*dy <= r*r })
}

// triangle draws an isoceles up/down triangle, filled or as an outline band —
// the trade layer's side-by-shape ladder (small/large × hollow/filled).
func (a *Atlas) triangle(rf float64, up, filled bool) []byte {
	// In "up" space: apex at dy=-r, base at dy=+r, half-width (dy+r)/2.
	in := func(dx, dy, r float64) bool {
		return dy >= -r && dy <= r && math.Abs(dx) <= (dy+r)/2
	}
	return a.shape(rf, func(dx, dy, r float64) bool {
		if !up {
			dy = -dy
		}
		if !in(dx, dy, r) {
			return false
		}
		if filled {
			return true
		}
		// Hollow: cut a copy scaled about the centroid (dy = r/3), leaving
		// an outline band ~r/3 wide — readable at the tiny cell size.
		const k = 1.5
		cy := r / 3
		return !in(dx*k, (dy-cy)*k+cy, r)
	})
}

func (a *Atlas) hline() []byte {
	mid := a.cellH / 2
	return a.fill(func(x, y int) uint8 {
		if y == mid {
			return 255
		}
		return 0
	})
}

func (a *Atlas) cross() []byte {
	midY, midX := a.cellH/2, a.cellW/2
	return a.fill(func(x, y int) uint8 {
		if y == midY || x == midX {
			return 255
		}
		return 0
	})
}

func (a *Atlas) dash() []byte {
	midY := a.cellH / 2
	lo, hi := a.cellW*15/100, a.cellW*85/100
	return a.fill(func(x, y int) uint8 {
		if y == midY && x >= lo && x < hi {
			return 255
		}
		return 0
	})
}
