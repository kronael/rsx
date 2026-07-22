package soft_test

import (
	"bytes"
	"flag"
	"image"
	"image/png"
	"os"
	"path/filepath"
	"testing"

	"rsx-glass"
	"rsx-glass/soft"
)

var update = flag.Bool("update", false, "rewrite golden PNGs")

const gridW, gridH = 72, 24

// richScene advances the deterministic synthetic feed far enough to fill the
// far tiers and cross the 30 s persistence threshold — a full heatmap: mid
// band, bid/ask walls, trades, persistence, exact ladders, ~mark.
func richScene() glass.Scene {
	s := glass.NewSynth(gridH)
	for i := 0; i < 60; i++ {
		s.Advance(5_000_000_000) // 5 s bins so the far windows fill quickly
	}
	return s.Scene()
}

// degradedScene is the offline, no-book state: the honesty markers (— for the
// unknown mid and empty ladders, ~mark —) and the amber offline link.
func degradedScene() glass.Scene {
	s := glass.NewSynth(gridH)
	s.Connected = false
	return s.Scene()
}

func renderGrid(t *testing.T, s glass.Scene) *image.RGBA {
	t.Helper()
	g := glass.NewGrid(gridW, gridH)
	glass.Compose(g, s)
	r, err := soft.New()
	if err != nil {
		t.Fatalf("soft.New: %v", err)
	}
	if err := r.Present(g); err != nil {
		t.Fatalf("Present: %v", err)
	}
	return r.Image()
}

// goldenPNG byte-locks a rendered frame against a committed PNG by comparing
// DECODED pixels (robust to PNG-encoder drift). Regenerate with -update.
func goldenPNG(t *testing.T, name string, got *image.RGBA) {
	t.Helper()
	path := filepath.Join("testdata", name)
	if *update {
		if err := os.MkdirAll("testdata", 0o755); err != nil {
			t.Fatal(err)
		}
		var buf bytes.Buffer
		if err := png.Encode(&buf, got); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read golden (run with -update to create): %v", err)
	}
	want, err := png.Decode(bytes.NewReader(raw))
	if err != nil {
		t.Fatalf("decode golden: %v", err)
	}
	wantRGBA, ok := want.(*image.RGBA)
	if !ok {
		b := want.Bounds()
		wantRGBA = image.NewRGBA(b)
		for y := b.Min.Y; y < b.Max.Y; y++ {
			for x := b.Min.X; x < b.Max.X; x++ {
				wantRGBA.Set(x, y, want.At(x, y))
			}
		}
	}
	if got.Bounds() != wantRGBA.Bounds() {
		t.Fatalf("%s: bounds %v, want %v", name, got.Bounds(), wantRGBA.Bounds())
	}
	if !bytes.Equal(got.Pix, wantRGBA.Pix) {
		t.Fatalf("%s drifted from golden (regenerate deliberately with -update)", name)
	}
}

func TestGoldenHeatmap(t *testing.T) {
	goldenPNG(t, "heatmap.png", renderGrid(t, richScene()))
}

func TestGoldenDegraded(t *testing.T) {
	goldenPNG(t, "degraded.png", renderGrid(t, degradedScene()))
}

// TestPresentAllocFree locks the RSX hot-path discipline: rasterising an
// already-built grid allocates nothing, so the GC never runs on the render loop.
func TestPresentAllocFree(t *testing.T) {
	g := glass.NewGrid(gridW, gridH)
	glass.Compose(g, richScene())
	r, err := soft.New()
	if err != nil {
		t.Fatalf("soft.New: %v", err)
	}
	if err := r.Present(g); err != nil { // warm up (allocates the framebuffer once)
		t.Fatal(err)
	}
	allocs := testing.AllocsPerRun(200, func() { _ = r.Present(g) })
	if allocs != 0 {
		t.Fatalf("Present allocates %.0f times per frame, want 0", allocs)
	}
}
