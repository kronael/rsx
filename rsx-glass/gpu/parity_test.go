package gpu_test

import (
	"bytes"
	"image"
	"image/png"
	"os"
	"runtime"
	"testing"

	"rsx-glass"
	"rsx-glass/gpu"
	"rsx-glass/soft"
)

const gridW, gridH = 72, 24

// richGrid builds the same full heatmap Grid the soft golden locks (mid band,
// bid/ask walls, trades, persistence, exact ladders) — the two renderers must
// draw THIS same Grid to the same pixels.
func richGrid() *glass.Grid {
	s := glass.NewSynth(gridH)
	for i := 0; i < 60; i++ {
		s.Advance(5_000_000_000)
	}
	g := glass.NewGrid(gridW, gridH)
	glass.Compose(g, s.Scene())
	return g
}

// TestGPUSoftParity is the renderer-parity check: the GPU backend rasterises the
// same Grid as the soft reference and must match it within tolerance. The only
// expected divergence is the last bit of colour rounding — soft blends in
// integer sRGB, the GPU in float — so a handful of channels differ by 1.
//
// It needs an off-screen GL context. On a headless box with no display (plain
// `go test ./...`) NewOffscreen fails and the test SKIPS with the reason; run it
// under a virtual display (e.g. `xvfb-run go test ./gpu/`) or on a desktop to
// actually compare pixels. It never fakes a pass.
func TestGPUSoftParity(t *testing.T) {
	runtime.LockOSThread() // GL needs a fixed OS thread; the constructors no longer lock for you
	off, err := gpu.NewOffscreen(gridW, gridH)
	if err != nil {
		t.Skipf("no off-screen GL context (%v); the gpu backend is compile-verified here and pixel-verified under a display", err)
	}
	defer off.Close() //nolint:errcheck // test teardown; nothing to recover

	g := richGrid()
	got := off.Render(g)

	ref, err := soft.New()
	if err != nil {
		t.Fatalf("soft.New: %v", err)
	}
	if err := ref.Present(g); err != nil {
		t.Fatalf("soft.Present: %v", err)
	}
	want := ref.Image()

	if got.Bounds() != want.Bounds() {
		t.Fatalf("bounds differ: gpu %v, soft %v", got.Bounds(), want.Bounds())
	}

	maxDiff, over1, chans := 0, 0, 0
	for i := 0; i+3 < len(want.Pix); i += 4 {
		for c := 0; c < 3; c++ { // RGB; alpha is a constant 0xff both sides
			d := int(want.Pix[i+c]) - int(got.Pix[i+c])
			if d < 0 {
				d = -d
			}
			if d > maxDiff {
				maxDiff = d
			}
			if d > 1 {
				over1++
			}
			chans++
		}
	}
	t.Logf("parity over %d RGB channels: maxDiff=%d, >1=%d (%.3f%%)",
		chans, maxDiff, over1, 100*float64(over1)/float64(chans))

	// Soft blends in integer sRGB, the GPU in float; that rounding differs by at
	// most ONE level, and no framebuffer sRGB conversion is enabled on either
	// side. That is the whole expected gap — measured maxDiff=1, zero channels
	// off by more than 1. Gate at exactly that measured bound; anything larger is
	// a real divergence (a shader, atlas, or colour-space bug), not rounding.
	if maxDiff > gpuParityMaxDiff {
		t.Errorf("gpu diverged from soft: maxDiff=%d > %d (structural, not sRGB rounding)", maxDiff, gpuParityMaxDiff)
	}

	if path := os.Getenv("RSX_GLASS_GPU_DUMP"); path != "" {
		writePNG(t, path, got)
	}
}

// gpuParityMaxDiff is the measured sRGB float-vs-integer rounding bound: one level.
const gpuParityMaxDiff = 1

func writePNG(t *testing.T, path string, img *image.RGBA) {
	t.Helper()
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
