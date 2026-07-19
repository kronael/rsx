# rsx-glass

The **GPU/desktop renderer for the rsx-term visual language** — the log-time
liquidity heatmap (a text Bookmap) drawn as *pixels* instead of terminal cells,
so it can render at arbitrary resolution and scale to high refresh.

It is **not** a rewrite of rsx-term: the model (order book, heatmap binning,
fisheye, honesty rules) is rsx-term's, imported and reused in-language. rsx-glass
replaces only the last stage — `cells → ANSI` becomes `cells → pixels`.

## The one idea: a movable render seam

```
rsx-term model (reused) → Grid (bg/fg/glyph cells) → Renderer → screen
                                 ^ plain data           ^ the only place GPU code lives
```

`Renderer` (`render.go`) is a two-method interface (`Present`, `Close`). Two
backends implement it:

- **`soft`** — a pure-Go software renderer (`Grid → image.RGBA`). The reference
  and **regression oracle** (it locks that the render did not *change*, not that
  the picture is *correct*), headless (no GPU, no display), and the fallback.
  This is what the goldens lock and what runs in CI (`CGO_ENABLED=0`-clean).
- **`gpu`** — go-gl (OpenGL + GLFW), an instanced-quad cell grid + glyph atlas,
  vsync-on. The desktop path, behind a `//go:build cgo` tag (a non-cgo stub keeps
  headless builds green). The interface is *designed* to swap for go-webgpu
  later — one package behind the seam — but that is **unproven**: no second
  windowed backend has been built yet.

## See it (headless, no display)

```sh
go test ./...            # builds the model→Grid→soft path, checks the goldens
```

The golden `soft/testdata/heatmap.png` is the rsx-term heatmap rendered to
pixels — proof the visual system rebuilds from spec.

## Run it (desktop)

```sh
go run ./cmd/glass       # opens a window, streams the book (in-process mock feed)
RSX_MD_URL=ws://host/md go run ./cmd/glass   # …or subscribe a real feed
```

`cmd/glass` is the composition root — the one place `gpu` is wired in, and it
owns `runtime.LockOSThread` (GLFW lifecycle + events + the GL context all live on
that one thread). The GPU backend draws the cell grid as an instanced-quad batch
(one `glDrawElementsInstanced` per frame) sampling the shared glyph atlas as a
texture, vsync-on; the render loop (`glass.go`) pumps window events every tick
(so the window stays closable — Esc/q quits — even before the first grid) and
hands the backend the newest `Grid` each tick, alloc-free.

The window is **resizable**: the fixed 72×24 grid is letterboxed at the largest
integer scale that fits, centred on a page-coloured margin, driven by the
framebuffer size (HiDPI-correct; NEAREST keeps the bitmap text crisp). True
reflow (re-deriving the grid from the size) is v2 — see SPEC "Known v1
deviations".

The `gpu` backend is held to the `soft` reference by a renderer-parity test
(`gpu/parity_test.go`): it renders the same `Grid` off-screen (FBO readback) and
must match soft within colour rounding (measured maxDiff = 1 level, no channel
off by more than 1). It needs a GL context, so it **skips** under headless CI and
runs on a desktop (or `xvfb-run go test ./gpu/`).

`gpu` is cgo (go-gl bundles the GLFW C source and links the platform GL/X11
libraries), so it needs a C toolchain + the dev headers. On Debian the X11 build
wants roughly `libgl1-mesa-dev libx11-dev libxrandr-dev libxinerama-dev
libxcursor-dev libxi-dev` (add `xvfb` + `libgl1-mesa-dri` to run the parity test
headless); the authoritative, binding-versioned list is go-gl/glfw's
[installation matrix](https://github.com/go-gl/glfw). A `CGO_ENABLED=0` build
compiles the non-cgo stub instead, so `go build`/`go test` stay green on a box
with no GPU (the `soft` path). On a Wayland-default desktop the app runs under
XWayland by default, where fractional scaling can blur the 1:1 pixel text; a
`wayland` go-gl build tag exists.

## Why Go (not Rust)

The whole rsx-term model is Go and works; in Go this is a render-backend swap,
not a port. The GC is a non-issue on an alloc-free render path (sub-100 µs
pauses can't drop a frame). See the repo's Go-vs-Rust benchmark. GPU-from-Go is
cgo (go-gl/go-webgpu), but at ~10 calls/frame the FFI cost is negligible — the
real cost is build/ship, not latency.

## How to read this crate

- **README.md** (this) — what/why/run.
- **SPEC.md** — the design + the movable seam + the test plan.
- **CLAUDE.md** — the conventions: rsx-term is the template, the keeper
  invariants (the seam, alloc-free, reuse, golden-lock).
- rsx-term's `VISUALS.md` / `SCREENS.md` / `notes/element-inventory.md` — the
  visual system this renders.
