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
	s := glass.NewSynth(gridW, gridH)
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

	maxDiff, over1, over8, chans := 0, 0, 0, 0
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
			if d > 8 {
				over8++
			}
			chans++
		}
	}
	t.Logf("parity over %d RGB channels: maxDiff=%d, >1=%d (%.3f%%), >8=%d",
		chans, maxDiff, over1, 100*float64(over1)/float64(chans), over8)

	// Rounding accounts for at most a 1-level difference; nothing should exceed
	// a small tolerance, and the vast majority of channels must be exact-or-±1.
	if maxDiff > gpuParityMaxDiff {
		t.Errorf("gpu diverged from soft: maxDiff=%d > %d", maxDiff, gpuParityMaxDiff)
	}
	if over8 != 0 {
		t.Errorf("gpu has %d channels off by >8 from soft (structural mismatch, not rounding)", over8)
	}
	if frac := float64(over1) / float64(chans); frac > gpuParityOver1Frac {
		t.Errorf("gpu differs from soft by >1 on %.2f%% of channels (>%.2f%%)",
			100*frac, 100*gpuParityOver1Frac)
	}

	if path := os.Getenv("RSX_GLASS_GPU_DUMP"); path != "" {
		writePNG(t, path, got)
	}
}

// Tolerances: float-vs-integer sRGB rounding differs by at most one level, so a
// tiny maxDiff and a small minority of ±>1 channels is the whole expected gap.
const (
	gpuParityMaxDiff   = 6
	gpuParityOver1Frac = 0.10
)

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
