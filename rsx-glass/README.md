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

- **`soft`** — a pure-Go software renderer (`Grid → image.RGBA`). The reference,
  the test oracle, headless (no GPU, no display), and the fallback. This is what
  the goldens lock and what runs in CI.
- **`gpu`** — go-gl (OpenGL + GLFW), an instanced-quad cell grid + glyph atlas,
  vsync-on. The desktop path. Swapping it for go-webgpu (Mailbox) is one package
  behind the same interface — the app never imports go-gl directly.

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

`cmd/glass` is the composition root — the one place `gpu` is wired in. The GPU
backend draws the cell grid as an instanced-quad batch (one `glDrawElementsInstanced`
per frame) sampling the shared glyph atlas as a texture, vsync-on; the render
loop (`glass.go`) hands it the newest `Grid` each tick, alloc-free.

The `gpu` backend is held to the `soft` reference by a renderer-parity test
(`gpu/parity_test.go`): it renders the same `Grid` off-screen (FBO readback) and
must match soft within colour rounding (measured maxDiff = 1 level, no channel
off by more than 1). It needs a GL context, so it **skips** under headless CI and
runs on a desktop (or `xvfb-run go test ./gpu/`).

`gpu` is cgo: building it needs the OpenGL + GLFW dev headers. On Debian:
`apt install libgl1-mesa-dev libglfw3-dev libx11-dev libxrandr-dev libxinerama-dev
libxcursor-dev libxi-dev` (add `xvfb` + `libgl1-mesa-dri` to run the parity test
headless).

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
