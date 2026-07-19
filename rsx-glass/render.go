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
	// Present draws g to the backend's target. For a vsync-on backend it also
	// blocks until the frame is shown, which paces the loop (present mode is a
	// knob inside the backend, not the loop's concern).
	Present(g *Grid) error
	// Close releases the backend's resources.
	Close() error
}
