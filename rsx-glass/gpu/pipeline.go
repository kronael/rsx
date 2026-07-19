package gpu

import (
	"fmt"
	"strings"

	"github.com/go-gl/gl/v3.3-core/gl"

	"rsx-glass"
	"rsx-glass/atlas"
)

// The instanced-quad pipeline: the whole Grid is drawn as ONE unit quad drawn
// once per cell (glDrawElementsInstanced). A per-cell instance carries the cell
// column/row, its resolved bg/fg colours, and the tile index of its glyph in the
// atlas texture; the vertex shader places the quad and picks the glyph's UV, the
// fragment shader composites fg over bg by the glyph's coverage — the exact
// blend soft does, in float. The atlas is uploaded once; per frame only the
// instance buffer is refilled and re-uploaded (alloc-free after warm-up), then a
// single draw call. This is the direct GPU analogue of soft's per-cell blit, so
// the two backends draw the same picture.
//
// A pipeline is context-agnostic: it assumes a current GL context and a bound
// framebuffer + viewport the caller has set (a window for the Renderer, an FBO
// for the offscreen parity path). That split is why one pipeline serves both.

// floatsPerInstance packs one cell: col,row, bg rgb, fg rgb, glyph tile index.
const floatsPerInstance = 9

const vertSrc = `#version 330 core
layout(location=0) in vec2 corner;   // unit-quad corner, {0,1}x{0,1}
layout(location=1) in vec2 cell;     // this cell's column,row
layout(location=2) in vec3 bg;       // resolved background, 0..1
layout(location=3) in vec3 fg;       // resolved foreground, 0..1
layout(location=4) in float tile;    // glyph's tile index in the atlas strip
uniform vec2 uGrid;                  // grid size in cells (W,H)
uniform float uCols;                 // number of tiles in the atlas strip
out vec3 vBg;
out vec3 vFg;
out vec2 vUV;
void main() {
	vec2 p = (cell + corner) / uGrid;              // 0..1, y downward (row 0 top)
	gl_Position = vec4(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
	vBg = bg;
	vFg = fg;
	vUV = vec2((tile + corner.x) / uCols, corner.y);
}
` + "\x00"

const fragSrc = `#version 330 core
in vec3 vBg;
in vec3 vFg;
in vec2 vUV;
uniform sampler2D uAtlas;            // R8 coverage, one horizontal strip of tiles
out vec4 frag;
void main() {
	float cov = texture(uAtlas, vUV).r;
	frag = vec4(vBg + (vFg - vBg) * cov, 1.0);   // soft's straight-sRGB blend
}
` + "\x00"

// pipeline holds the GL objects and the reused instance staging buffer.
type pipeline struct {
	prog    uint32
	vao     uint32
	quadVBO uint32
	ebo     uint32
	instVBO uint32
	tex     uint32

	uGrid  int32
	uCols  int32
	uAtlas int32

	cw    int
	ch    int
	cols  int32          // atlas tiles (tile 0 is blank)
	index map[rune]int32 // rune -> tile index; unknown -> 0 (blank)
	inst  []float32      // reused per-frame instance staging (grows only on resize)
}

// newPipeline compiles the shaders, packs the atlas into one texture, and
// allocates the static quad. The GL context must already be current.
func newPipeline(a *atlas.Atlas) (*pipeline, error) {
	prog, err := buildProgram(vertSrc, fragSrc)
	if err != nil {
		return nil, err
	}
	cw, ch := a.CellSize()
	p := &pipeline{prog: prog, cw: cw, ch: ch, index: map[rune]int32{}}
	p.uGrid = gl.GetUniformLocation(prog, gl.Str("uGrid\x00"))
	p.uCols = gl.GetUniformLocation(prog, gl.Str("uCols\x00"))
	p.uAtlas = gl.GetUniformLocation(prog, gl.Str("uAtlas\x00"))

	p.uploadAtlas(a)
	p.buildQuad()
	return p, nil
}

// uploadAtlas packs every glyph mask into a single R8 texture laid out as a
// horizontal strip of cw x ch tiles (tile 0 blank), and records rune -> tile.
func (p *pipeline) uploadAtlas(a *atlas.Atlas) {
	runes := a.Glyphs()
	p.cols = int32(len(runes) + 1) // tile 0 reserved as the blank/unknown tile
	stripW := int(p.cols) * p.cw
	strip := make([]byte, stripW*p.ch)
	for i, r := range runes {
		tile := i + 1
		p.index[r] = int32(tile)
		m := a.Mask(r)
		if m == nil {
			continue // blank glyph (space): leave the tile zeroed
		}
		for y := 0; y < p.ch; y++ {
			dst := y*stripW + tile*p.cw
			copy(strip[dst:dst+p.cw], m[y*p.cw:(y+1)*p.cw])
		}
	}

	gl.GenTextures(1, &p.tex)
	gl.BindTexture(gl.TEXTURE_2D, p.tex)
	gl.PixelStorei(gl.UNPACK_ALIGNMENT, 1)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
	gl.TexImage2D(gl.TEXTURE_2D, 0, gl.R8, int32(stripW), int32(p.ch), 0, gl.RED, gl.UNSIGNED_BYTE, gl.Ptr(strip))
}

// buildQuad uploads the static unit quad + element indices and wires the vertex
// layout: corner is per-vertex, the cell/bg/fg/tile attributes are per-instance.
func (p *pipeline) buildQuad() {
	corners := []float32{0, 0, 1, 0, 0, 1, 1, 1}
	indices := []uint32{0, 1, 2, 2, 1, 3}

	gl.GenVertexArrays(1, &p.vao)
	gl.BindVertexArray(p.vao)

	gl.GenBuffers(1, &p.quadVBO)
	gl.BindBuffer(gl.ARRAY_BUFFER, p.quadVBO)
	gl.BufferData(gl.ARRAY_BUFFER, len(corners)*4, gl.Ptr(corners), gl.STATIC_DRAW)
	gl.EnableVertexAttribArray(0)
	gl.VertexAttribPointerWithOffset(0, 2, gl.FLOAT, false, 2*4, 0)

	gl.GenBuffers(1, &p.ebo)
	gl.BindBuffer(gl.ELEMENT_ARRAY_BUFFER, p.ebo)
	gl.BufferData(gl.ELEMENT_ARRAY_BUFFER, len(indices)*4, gl.Ptr(indices), gl.STATIC_DRAW)

	stride := int32(floatsPerInstance * 4)
	gl.GenBuffers(1, &p.instVBO)
	gl.BindBuffer(gl.ARRAY_BUFFER, p.instVBO)
	for _, at := range []struct {
		loc, size uint32
		offset    uintptr
	}{
		{1, 2, 0},  // cell col,row
		{2, 3, 8},  // bg rgb
		{3, 3, 20}, // fg rgb
		{4, 1, 32}, // tile
	} {
		gl.EnableVertexAttribArray(at.loc)
		gl.VertexAttribPointerWithOffset(at.loc, int32(at.size), gl.FLOAT, false, stride, at.offset)
		gl.VertexAttribDivisor(at.loc, 1) // advance once per instance (cell), not per vertex
	}
	gl.BindVertexArray(0)
}

// draw renders g into the currently bound framebuffer. The caller owns the
// framebuffer, the viewport, and buffer-swapping/readback — draw only issues the
// upload + single instanced draw call. Alloc-free once inst is sized.
func (p *pipeline) draw(g *glass.Grid) {
	need := g.W * g.H * floatsPerInstance
	grew := false
	if cap(p.inst) < need {
		p.inst = make([]float32, need)
		grew = true
	}
	p.inst = p.inst[:need]
	n := 0
	for cy := 0; cy < g.H; cy++ {
		for cx := 0; cx < g.W; cx++ {
			c := g.Cells[cy*g.W+cx]
			b := n * floatsPerInstance
			p.inst[b+0] = float32(cx)
			p.inst[b+1] = float32(cy)
			p.inst[b+2] = float32(c.Bg.R) / 255
			p.inst[b+3] = float32(c.Bg.G) / 255
			p.inst[b+4] = float32(c.Bg.B) / 255
			p.inst[b+5] = float32(c.Fg.R) / 255
			p.inst[b+6] = float32(c.Fg.G) / 255
			p.inst[b+7] = float32(c.Fg.B) / 255
			p.inst[b+8] = float32(p.tileOf(c.Glyph))
			n++
		}
	}

	gl.BindBuffer(gl.ARRAY_BUFFER, p.instVBO)
	if grew {
		gl.BufferData(gl.ARRAY_BUFFER, need*4, gl.Ptr(p.inst), gl.DYNAMIC_DRAW)
	} else {
		gl.BufferSubData(gl.ARRAY_BUFFER, 0, need*4, gl.Ptr(p.inst))
	}

	gl.Disable(gl.BLEND)
	gl.Disable(gl.DEPTH_TEST)
	gl.ClearColor(float32(glass.Page.R)/255, float32(glass.Page.G)/255, float32(glass.Page.B)/255, 1)
	gl.Clear(gl.COLOR_BUFFER_BIT)

	gl.UseProgram(p.prog)
	gl.Uniform2f(p.uGrid, float32(g.W), float32(g.H))
	gl.Uniform1f(p.uCols, float32(p.cols))
	gl.ActiveTexture(gl.TEXTURE0)
	gl.BindTexture(gl.TEXTURE_2D, p.tex)
	gl.Uniform1i(p.uAtlas, 0)
	gl.BindVertexArray(p.vao)
	gl.DrawElementsInstanced(gl.TRIANGLES, 6, gl.UNSIGNED_INT, nil, int32(n))
	gl.BindVertexArray(0)
}

// tileOf maps a glyph rune to its atlas tile, defaulting to the blank tile 0 for
// space and any rune the atlas has no mask for (soft draws those as bg too).
func (p *pipeline) tileOf(r rune) int32 { return p.index[r] }

// pixelSize is the framebuffer size a grid rasterises to at the atlas cell size.
func (p *pipeline) pixelSize(g *glass.Grid) (w, h int) { return g.W * p.cw, g.H * p.ch }

// release deletes the GL objects (call with the context current).
func (p *pipeline) release() {
	gl.DeleteTextures(1, &p.tex)
	gl.DeleteBuffers(1, &p.instVBO)
	gl.DeleteBuffers(1, &p.ebo)
	gl.DeleteBuffers(1, &p.quadVBO)
	gl.DeleteVertexArrays(1, &p.vao)
	gl.DeleteProgram(p.prog)
}

// buildProgram compiles and links the vertex + fragment shaders.
func buildProgram(vs, fs string) (uint32, error) {
	v, err := compileShader(vs, gl.VERTEX_SHADER)
	if err != nil {
		return 0, err
	}
	f, err := compileShader(fs, gl.FRAGMENT_SHADER)
	if err != nil {
		return 0, err
	}
	prog := gl.CreateProgram()
	gl.AttachShader(prog, v)
	gl.AttachShader(prog, f)
	gl.LinkProgram(prog)
	gl.DeleteShader(v)
	gl.DeleteShader(f)
	var ok int32
	gl.GetProgramiv(prog, gl.LINK_STATUS, &ok)
	if ok == gl.FALSE {
		var n int32
		gl.GetProgramiv(prog, gl.INFO_LOG_LENGTH, &n)
		log := strings.Repeat("\x00", int(n)+1)
		gl.GetProgramInfoLog(prog, n, nil, gl.Str(log))
		return 0, fmt.Errorf("gpu: link program: %s", log)
	}
	return prog, nil
}

// compileShader compiles one shader stage, returning its GL name.
func compileShader(src string, kind uint32) (uint32, error) {
	sh := gl.CreateShader(kind)
	csrc, free := gl.Strs(src)
	gl.ShaderSource(sh, 1, csrc, nil)
	free()
	gl.CompileShader(sh)
	var ok int32
	gl.GetShaderiv(sh, gl.COMPILE_STATUS, &ok)
	if ok == gl.FALSE {
		var n int32
		gl.GetShaderiv(sh, gl.INFO_LOG_LENGTH, &n)
		log := strings.Repeat("\x00", int(n)+1)
		gl.GetShaderInfoLog(sh, n, nil, gl.Str(log))
		return 0, fmt.Errorf("gpu: compile shader: %s", log)
	}
	return sh, nil
}
