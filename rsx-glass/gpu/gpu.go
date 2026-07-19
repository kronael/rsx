//go:build cgo

// Package gpu is the OpenGL desktop backend: the second real glass.Renderer
// behind the movable seam. It draws the exact same picture as soft — it shares
// the atlas package's glyph masks and mirrors soft's cell layout and palette —
// but on the GPU as an instanced-quad cell grid (one glDrawElementsInstanced per
// frame, vsync-on). This is the ONLY package allowed to import go-gl; the app
// depends on glass.Renderer and wires this in at its composition root alone.
//
// The GL pipeline (package-private, pipeline.go) is context-agnostic. Two
// contexts drive it: Renderer opens a real window with GLFW (the desktop path),
// and Offscreen renders into an FBO for readback (the headless parity test).
// Swapping GLFW/OpenGL for go-webgpu/Mailbox later replaces only this file.
package gpu

import (
	"errors"
	"fmt"
	"image"

	"github.com/go-gl/gl/v3.3-core/gl"
	"github.com/go-gl/glfw/v3.3/glfw"

	"rsx-glass"
	"rsx-glass/atlas"
)

// ErrClosed is returned by Present once the user closes the window. glass.Run
// treats a Present error as a clean stop, so this ends the render loop.
var ErrClosed = errors.New("gpu: window closed")

// Renderer is the windowed OpenGL backend. It satisfies glass.Renderer, so the
// loop in glass.Run drives it exactly like soft. Present is vsync-paced (the
// present mode is a knob here, not the loop's concern) and alloc-free after the
// first frame — only the reused instance buffer is re-uploaded.
type Renderer struct {
	win  *glfw.Window
	pipe *pipeline
}

// New opens a window sized to a gridW x gridH grid at the atlas cell size and
// returns a Renderer with vsync on. The window is resizable; the fixed grid is
// letterboxed inside it (see Present), and a minimum size keeps it at >= 1:1.
//
// New must be called on an OS-thread-locked goroutine — the process main thread.
// GLFW's lifecycle and event calls are main-thread-only, and this backend's GL
// context is current on that one thread; Pump/Present/Close must run there too.
// New does NOT lock the thread itself: a library constructor cannot know it was
// called on the main thread, so the composition root owns runtime.LockOSThread
// (see cmd/glass/main.go). glfw.Init/Terminate are process-global — v1 runs one
// windowed Renderer per process.
func New(gridW, gridH int, title string) (*Renderer, error) {
	a, err := atlas.New()
	if err != nil {
		return nil, err
	}
	cw, ch := a.CellSize()
	win, err := newWindow(gridW*cw, gridH*ch, title, true)
	if err != nil {
		return nil, err
	}
	win.SetSizeLimits(gridW*cw, gridH*ch, glfw.DontCare, glfw.DontCare) // never below 1:1
	win.MakeContextCurrent()
	if err := gl.Init(); err != nil {
		win.Destroy()
		glfw.Terminate()
		return nil, fmt.Errorf("gpu: gl.Init: %w", err)
	}
	glfw.SwapInterval(1) // vsync-on: Present blocks until the frame is shown
	pipe, err := newPipeline(a)
	if err != nil {
		win.Destroy()
		glfw.Terminate()
		return nil, err
	}
	return &Renderer{win: win, pipe: pipe}, nil
}

// Pump processes pending window events and returns ErrClosed once the user asks
// to close the window (the close button or the quit key). glass.Run calls it
// every tick, independent of data, so the window stays responsive even before
// the first grid arrives or while the feed is down.
func (r *Renderer) Pump() error {
	glfw.PollEvents()
	if r.win.ShouldClose() {
		return ErrClosed
	}
	return nil
}

// Present draws g to the window and swaps buffers, blocking on vsync. Events are
// pumped by Pump, so Present only renders — it is called after a Pump that did
// not report a close.
//
// The fixed grid is LETTERBOXED: it draws at the largest integer multiple of its
// native pixel size that fits the framebuffer, centred, on a Page-coloured
// margin (draw clears the whole framebuffer to Page first). The FRAMEBUFFER size
// in pixels — not the window size in screen coordinates — drives the viewport,
// so it is correct on HiDPI/fractional-scaled displays; integer scaling with
// NEAREST keeps the bitmap text crisp. A 0-sized framebuffer (minimized) draws
// nothing. True reflow (re-deriving grid W×H from the size) is v2.
func (r *Renderer) Present(g *glass.Grid) error {
	fbW, fbH := r.win.GetFramebufferSize()
	if fbW <= 0 || fbH <= 0 { // minimized: nothing visible to draw
		return nil
	}
	natW, natH := r.pipe.pixelSize(g)
	scale := fbW / natW
	if s := fbH / natH; s < scale {
		scale = s
	}
	if scale < 1 {
		scale = 1 // the minimum window size holds us at >= 1:1; clamp defends the math
	}
	vpW, vpH := natW*scale, natH*scale
	gl.Viewport(int32((fbW-vpW)/2), int32((fbH-vpH)/2), int32(vpW), int32(vpH))
	r.pipe.draw(g)
	r.win.SwapBuffers()
	return nil
}

// Close releases the pipeline, destroys the window, and terminates GLFW. It is
// idempotent: a second call is a no-op. glfw.Terminate is process-global, so
// this ends GLFW for the process (v1: one windowed Renderer per process).
func (r *Renderer) Close() error {
	if r.win == nil {
		return nil
	}
	if r.pipe != nil {
		r.pipe.release()
		r.pipe = nil
	}
	r.win.Destroy()
	r.win = nil
	glfw.Terminate()
	return nil
}

// Offscreen renders grids into an off-screen framebuffer and reads the pixels
// back, with no visible window — the headless path the parity test uses to
// compare the GPU backend against the soft reference. It shares the exact same
// pipeline as Renderer, so it verifies the shipping draw path, not a stand-in.
type Offscreen struct {
	win  *glfw.Window
	pipe *pipeline
	fbo  uint32
	tex  uint32
	w    int
	h    int
	buf  []byte      // reused readback staging (bottom-up)
	img  *image.RGBA // reused flipped result (top-down)
}

// NewOffscreen builds a hidden GL context and an FBO sized to gridW x gridH at
// the atlas cell size. It returns an error (not a panic) when no GL context is
// available, so the parity test can skip cleanly on a headless box with no
// display. Like New, it must be called on an OS-thread-locked goroutine and does
// NOT lock itself — the parity test locks its own test goroutine.
func NewOffscreen(gridW, gridH int) (*Offscreen, error) {
	a, err := atlas.New()
	if err != nil {
		return nil, err
	}
	cw, ch := a.CellSize()
	pxW, pxH := gridW*cw, gridH*ch
	win, err := newWindow(pxW, pxH, "", false)
	if err != nil {
		return nil, err
	}
	win.MakeContextCurrent()
	if err := gl.Init(); err != nil {
		win.Destroy()
		glfw.Terminate()
		return nil, fmt.Errorf("gpu: gl.Init: %w", err)
	}
	pipe, err := newPipeline(a)
	if err != nil {
		win.Destroy()
		glfw.Terminate()
		return nil, err
	}
	o := &Offscreen{
		win:  win,
		pipe: pipe,
		w:    pxW,
		h:    pxH,
		buf:  make([]byte, pxW*pxH*4),
		img:  image.NewRGBA(image.Rect(0, 0, pxW, pxH)),
	}
	if err := o.attachFBO(); err != nil {
		o.Close()
		return nil, err
	}
	return o, nil
}

// attachFBO creates the color texture + framebuffer the pipeline renders into.
func (o *Offscreen) attachFBO() error {
	gl.GenFramebuffers(1, &o.fbo)
	gl.BindFramebuffer(gl.FRAMEBUFFER, o.fbo)
	gl.GenTextures(1, &o.tex)
	gl.BindTexture(gl.TEXTURE_2D, o.tex)
	gl.TexImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, int32(o.w), int32(o.h), 0, gl.RGBA, gl.UNSIGNED_BYTE, nil)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
	gl.TexParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
	gl.FramebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, o.tex, 0)
	if s := gl.CheckFramebufferStatus(gl.FRAMEBUFFER); s != gl.FRAMEBUFFER_COMPLETE {
		return fmt.Errorf("gpu: framebuffer incomplete: 0x%x", s)
	}
	return nil
}

// Render rasterises g into the off-screen buffer and returns the pixels as an
// *image.RGBA (top-down, like soft). The result buffer is reused across calls.
func (o *Offscreen) Render(g *glass.Grid) *image.RGBA {
	gl.BindFramebuffer(gl.FRAMEBUFFER, o.fbo)
	gl.Viewport(0, 0, int32(o.w), int32(o.h))
	o.pipe.draw(g)
	gl.Finish()
	gl.PixelStorei(gl.PACK_ALIGNMENT, 1)
	gl.ReadPixels(0, 0, int32(o.w), int32(o.h), gl.RGBA, gl.UNSIGNED_BYTE, gl.Ptr(o.buf))
	// glReadPixels returns rows bottom-to-top; flip to soft's top-down order.
	stride := o.w * 4
	for y := 0; y < o.h; y++ {
		src := (o.h - 1 - y) * stride
		copy(o.img.Pix[y*stride:y*stride+stride], o.buf[src:src+stride])
	}
	return o.img
}

// Close releases the FBO, the pipeline, the window, and GLFW. It is idempotent.
func (o *Offscreen) Close() error {
	if o.win == nil {
		return nil
	}
	if o.fbo != 0 {
		gl.DeleteFramebuffers(1, &o.fbo)
		o.fbo = 0
	}
	if o.tex != 0 {
		gl.DeleteTextures(1, &o.tex)
		o.tex = 0
	}
	if o.pipe != nil {
		o.pipe.release()
		o.pipe = nil
	}
	o.win.Destroy()
	o.win = nil
	glfw.Terminate()
	return nil
}

// newWindow initialises GLFW and creates a 3.3-core-profile window (visible for
// the desktop path, hidden for the offscreen path). On a box with no display
// GLFW fails to init, but go-gl/glfw reports that failure by logging the platform
// error and then panicking NotInitialized on the next call (go-gl/glfw#127), so
// the deferred recover turns a headless box into a clean error the caller skips
// on — never a crash.
func newWindow(w, h int, title string, visible bool) (win *glfw.Window, err error) {
	defer func() {
		if r := recover(); r != nil {
			win, err = nil, fmt.Errorf("gpu: glfw unavailable (no display?): %v", r)
		}
	}()
	if e := glfw.Init(); e != nil {
		return nil, fmt.Errorf("gpu: glfw.Init: %w", e)
	}
	glfw.WindowHint(glfw.ContextVersionMajor, 3)
	glfw.WindowHint(glfw.ContextVersionMinor, 3)
	glfw.WindowHint(glfw.OpenGLProfile, glfw.OpenGLCoreProfile)
	glfw.WindowHint(glfw.OpenGLForwardCompatible, glfw.True)
	if visible {
		glfw.WindowHint(glfw.Visible, glfw.True)
		glfw.WindowHint(glfw.Resizable, glfw.True) // resizable; Present letterboxes the fixed grid
	} else {
		glfw.WindowHint(glfw.Visible, glfw.False)
		glfw.WindowHint(glfw.Resizable, glfw.False)
	}
	window, e := glfw.CreateWindow(w, h, title, nil, nil)
	if e != nil {
		glfw.Terminate()
		return nil, fmt.Errorf("gpu: create window: %w", e)
	}
	if visible {
		window.SetKeyCallback(quitOnEscape)
	}
	return window, nil
}

// quitOnEscape flags the window to close on Esc or q — the read-only viewer's
// one input, so it is keyboard-closable, not just via the window chrome.
func quitOnEscape(w *glfw.Window, key glfw.Key, _ int, action glfw.Action, _ glfw.ModifierKey) {
	if action == glfw.Press && (key == glfw.KeyEscape || key == glfw.KeyQ) {
		w.SetShouldClose(true)
	}
}
