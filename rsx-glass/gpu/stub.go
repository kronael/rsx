//go:build !cgo

// The non-cgo stub for package gpu. go-gl needs cgo, a C toolchain, and GL/GLFW
// headers, so a CGO_ENABLED=0 build (headless CI, a box with no GPU) compiles
// THIS file instead of the OpenGL backend. Every entry point returns an error,
// so the composition root fails fast with a clear message and the parity test
// skips cleanly — the pure-Go soft path stays buildable and green everywhere.
package gpu

import (
	"errors"
	"image"

	"rsx-glass"
)

// ErrClosed mirrors the real backend's sentinel so callers that test for it
// (errors.Is) compile in either build.
var ErrClosed = errors.New("gpu: window closed")

// errNoCGO is what every constructor returns without cgo.
var errNoCGO = errors.New("gpu: built without cgo (the OpenGL backend needs CGO_ENABLED=1 and GL/GLFW headers)")

// Renderer is the stubbed windowed backend. New never returns one, so its
// methods are unreachable; they exist only to satisfy glass.Renderer.
type Renderer struct{}

// New reports that the GPU backend is unavailable in a non-cgo build.
func New(gridW, gridH int, title string) (*Renderer, error) { return nil, errNoCGO }

// Pump satisfies glass.Renderer (unreachable: New never succeeds here).
func (r *Renderer) Pump() error { return ErrClosed }

// Present satisfies glass.Renderer (unreachable).
func (r *Renderer) Present(*glass.Grid) error { return ErrClosed }

// Close satisfies glass.Renderer (unreachable).
func (r *Renderer) Close() error { return nil }

// Offscreen is the stubbed FBO path; NewOffscreen never returns one.
type Offscreen struct{}

// NewOffscreen reports no off-screen GL context, so the parity test skips.
func NewOffscreen(gridW, gridH int) (*Offscreen, error) { return nil, errNoCGO }

// Render satisfies the offscreen path (unreachable).
func (o *Offscreen) Render(*glass.Grid) *image.RGBA { return nil }

// Close satisfies the offscreen path (unreachable).
func (o *Offscreen) Close() error { return nil }
