// Package soft is the pure-Go software renderer: it turns a glass.Grid into an
// *image.RGBA with no cgo, no GPU, and no display. It is the REFERENCE and the
// test oracle (byte-locked golden PNGs) and the headless-verifiable path — the
// direct analogue of rsx-term's cells→ANSI, with a pixel target instead of a
// terminal. The GPU backend shares the same atlas, so their pixels agree.
package soft

import (
	"image"

	"rsx-glass"
	"rsx-glass/atlas"
)

// Renderer rasterises grids into a reused framebuffer. Present is alloc-free
// after the first frame (the atlas is built once, the framebuffer is reused),
// so the render loop never triggers the GC.
type Renderer struct {
	atlas *atlas.Atlas
	cellW int
	cellH int
	fb    *image.RGBA
}

// New builds the shared glyph atlas and an empty renderer.
func New() (*Renderer, error) {
	a, err := atlas.New()
	if err != nil {
		return nil, err
	}
	w, h := a.CellSize()
	return &Renderer{atlas: a, cellW: w, cellH: h}, nil
}

// CellSize returns the atlas cell dimensions in pixels.
func (r *Renderer) CellSize() (w, h int) { return r.cellW, r.cellH }

// Image returns the last framebuffer Present drew (nil before the first frame).
func (r *Renderer) Image() *image.RGBA { return r.fb }

// Present rasterises g into the framebuffer. It reallocates only when the pixel
// dimensions change (first frame or resize); steady-state frames allocate zero.
func (r *Renderer) Present(g *glass.Grid) error {
	w, h := g.W*r.cellW, g.H*r.cellH
	if r.fb == nil || r.fb.Rect.Dx() != w || r.fb.Rect.Dy() != h {
		r.fb = image.NewRGBA(image.Rect(0, 0, w, h))
	}
	for cy := 0; cy < g.H; cy++ {
		for cx := 0; cx < g.W; cx++ {
			r.blit(g.Cells[cy*g.W+cx], cx*r.cellW, cy*r.cellH)
		}
	}
	return nil
}

// Close releases resources (the software renderer holds none).
func (r *Renderer) Close() error { return nil }

// blit draws one cell at pixel origin (px0, py0): the background, with the
// glyph's coverage mask composited in the foreground over it.
func (r *Renderer) blit(cell glass.Cell, px0, py0 int) {
	mask := r.atlas.Mask(cell.Glyph)
	bg, fg := cell.Bg, cell.Fg
	pix, stride := r.fb.Pix, r.fb.Stride
	for y := 0; y < r.cellH; y++ {
		row := (py0+y)*stride + px0*4
		for x := 0; x < r.cellW; x++ {
			var cov int
			if mask != nil {
				cov = int(mask[y*r.cellW+x])
			}
			i := row + x*4
			if cov == 0 {
				pix[i] = bg.R
				pix[i+1] = bg.G
				pix[i+2] = bg.B
			} else {
				pix[i] = lerp(bg.R, fg.R, cov)
				pix[i+1] = lerp(bg.G, fg.G, cov)
				pix[i+2] = lerp(bg.B, fg.B, cov)
			}
			pix[i+3] = 0xff
		}
	}
}

// lerp blends a→b by coverage cov (0..255), straight-alpha in sRGB space (the
// same non-linear lerp rsx-term's blendHex uses — consistent with the terminal).
func lerp(a, b uint8, cov int) uint8 {
	return uint8(int(a) + (int(b)-int(a))*cov/255)
}
