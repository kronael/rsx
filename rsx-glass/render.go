package glass

// Renderer turns a Grid into pixels on some target — a window (package gpu), an
// in-memory framebuffer (package soft). It is the ONE thing a backend
// implements, and the ONLY place native/GPU code is allowed to live: the app
// depends on this interface, never on go-gl. Swapping OpenGL for go-webgpu is
// one package behind this seam.
//
// Present must be alloc-free after warm-up (RSX hot-path discipline: the GC
// never runs on the render loop) and is called with the NEWEST grid — a
// Renderer holds no scene graph, it just draws what it is handed.
type Renderer interface {
	// Pump processes pending window events (close, resize, keys) and reports a
	// backend-signalled stop (e.g. the window closed) as an error. The loop
	// calls it EVERY iteration, independent of data — a window must stay
	// closable and repaintable even while the feed is down. A backend with no
	// window (soft) returns nil.
	Pump() error
	// Present draws g to the backend's target. For a vsync-on backend it also
	// blocks until the frame is shown, which paces the loop (present mode is a
	// knob inside the backend, not the loop's concern). Present is called only
	// when there is a grid to draw; Pump runs regardless.
	Present(g *Grid) error
	// Close releases the backend's resources. It is idempotent.
	Close() error
}
