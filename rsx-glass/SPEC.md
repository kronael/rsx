# rsx-glass — SPEC

## What

The native desktop GPU renderer for the **rsx-term visual language**. Same
picture as the rsx-term TUI — the log-time liquidity heatmap as a grid of cells
— drawn on the GPU at arbitrary resolution, so it can scale to high refresh.
**All Go.** The GPU/native code is **concentrated behind one interface** so it
can be moved (go-gl ↔ go-webgpu ↔ a hand-written C leaf) without touching the
app.

## Non-negotiables (inherited from rsx-term — the template)

- **Minimal, like rsx-term.** Fewest moving parts. Prune to rsx-term's element
  set (`rsx-term/notes/element-inventory.md`).
- **The render is a PURE FUNCTION of the newest model state (a `Grid`).** No
  retained scene graph. Data-rate ⟂ render-rate: render on data-arrival or a
  fixed tick off the *latest* Grid.
- **Alloc-free per frame** (RSX hot-path discipline) → the GC never runs on the
  render loop. Enforced by a test.
- **Honesty preserved** (rsx-term's rules): `—` unknown, `~` estimate,
  hard-block over the fat-finger cap, aggregate rows labelled. The renderer
  never fabricates; it draws what the model gives.
- **Reuse rsx-term's model in-language.** rsx-glass consumes rsx-term's cell
  grid at the model→render seam; it does NOT re-implement book folds / heatmap
  binning / fisheye.

## Architecture — the movable seam

```
rsx-term model (reused)  →  Grid (cells: bg, fg, glyph)  →  Renderer  →  screen
                                    ^ plain data              ^ the movable leaf
```

- **`Grid` / `Cell`** — the plain-data contract (mirrors rsx-term's cell model
  per `rsx-term/VISUALS.md`). No behaviour. The stable boundary between the
  (reused) model and any renderer.
- **`Renderer`** interface — `Frame(g *Grid) *image.RGBA` (reference) and/or
  `Present(g *Grid)` (windowed). The ONE thing a backend implements.
- **Backends (concentrated, swappable):**
  1. **`soft`** — pure-Go software renderer (`Grid → image.RGBA`, glyph atlas
     via `x/image/font`). The REFERENCE + test oracle + headless-verifiable +
     fallback. No cgo, no GPU, no display. This is the direct analogue of
     rsx-term's `cells → ANSI` — same shape, pixel target instead of a terminal.
  2. **`gpu`** — go-gl (OpenGL + GLFW), vsync-on, instanced-quad cell grid +
     glyph atlas. The desktop path, behind the same interface. Swappable for
     go-webgpu/Mailbox later — that is the entire point of the seam.

## Present / loop

- **go-gl vsync-on** (per the latency analysis: for a data-driven ~100 ms-update
  heatmap, vsync-on ≈ Mailbox — you never outrun the display). Present mode is a
  KNOB; the backend is isolated so Mailbox (go-webgpu) is a swap, not a rewrite.
- Loop: `select { case g := <-grids: latest = g; case <-tick.C: }` → render
  `latest`, alloc-free.

## Reuse mechanism

- A `go.work` (or a local `replace`) so rsx-glass imports rsx-term's model
  packages directly.
- Expose the model→render seam in rsx-term if not already public: a
  `Cells() Grid` accessor on the streaming model — the exact boundary
  `rsx-term/CLAUDE.md`'s "Visual design system" already names ("a GPU/desktop
  successor replaces only the model→render boundary"). Minimal, additive; **must
  not regress rsx-term's DOM/book goldens**.

## Test plan (SHIP THE TESTS)

- **Golden-image tests** (rsx-term's DOM-golden discipline, ported): a set of
  representative `Grid`s (heatmap with a mid band, bid/ask walls, trades,
  honesty markers, degraded states) → byte-locked reference PNGs from the `soft`
  renderer. Locks the `Grid → pixels` contract; a regression is a failing
  golden.
- **Renderer-parity test**: the `gpu` backend renders the same `Grid`s (headless
  off-screen where the backend supports it) and must match the `soft` reference
  within tolerance. Where headless GPU isn't available, skip with a clear reason
  (runs on the desktop).
- **Model-reuse smoke test**: a real rsx-term model state produces a valid
  `Grid` (the seam works end-to-end).
- **Alloc test**: `testing.AllocsPerRun` on the render path == 0.
- Every headless-runnable test passes with **no display** (the `soft` path).

## Layout (minimal)

```
rsx-glass/
  grid.go          Cell, Grid (the contract)
  render.go        Renderer interface
  soft/            pure-Go software renderer + glyph atlas
  gpu/             go-gl backend (build-tagged; needs a desktop to run)
  glass.go         the loop (newest-Grid → render)
  main.go          the desktop binary
  *_test.go, testdata/*.png   golden tests
  SPEC.md, README.md, CLAUDE.md
```

## CLAUDE.md must record (load-bearing)

- **rsx-term is the template. Always.** Its minimalism, its visual design system
  (`rsx-term/VISUALS.md`, `SCREENS.md`, `notes/element-inventory.md` — the
  rebuild-from-this spec), its honesty rules, its model=pure-fn seam, and its
  golden-lock discipline are the law here. When in doubt, do what rsx-term does.
- **The movable-seam rule.** All GPU/native code stays behind `Renderer`; the
  app never imports go-gl/go-webgpu directly. Swapping the backend is one
  package.
- Alloc-free render; data ⟂ render rate; present mode is a knob.

## Out of scope (v1 — keep it minimal)

- **Input / trading.** v1 is a read-only viewer (the heatmap). Keyboard +
  order-entry is v2 (reuse rsx-term's keymap then).
- **Mailbox / go-webgpu.** The seam allows it later; v1 is go-gl vsync-on.
- **240 Hz tuning.** The seams are ready; not turned on.
