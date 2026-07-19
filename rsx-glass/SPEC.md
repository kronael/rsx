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
- **`Renderer`** interface — `Pump()` (always process window events),
  `Present(g *Grid)` (draw the newest grid), `Close()`. The ONE thing a backend
  implements; `soft` returns nil from Pump (no window).
- **Backends (concentrated, swappable):**
  1. **`soft`** — pure-Go software renderer (`Grid → image.RGBA`, glyph atlas
     via `x/image/font`). The REFERENCE + test oracle + headless-verifiable +
     fallback. No cgo, no GPU, no display. This is the direct analogue of
     rsx-term's `cells → ANSI` — same shape, pixel target instead of a terminal.
  2. **`gpu`** — go-gl (OpenGL + GLFW), vsync-on, instanced-quad cell grid +
     glyph atlas. The desktop path, behind the same interface. Swappable for
     go-webgpu/Mailbox later — that is the entire point of the seam.

## Present / loop

- **go-gl vsync-on.** `SwapInterval(1)` is a *request* to the driver, not a
  guarantee (a compositor or driver setting can override it), so we do not treat
  a returned `SwapBuffers` as "physically shown". For this dirty-only ~100 ms
  data feed vsync-on *may look* like Vulkan Mailbox — you rarely outrun the
  display — but that is a workload observation, not an equivalence: FIFO can
  still show an older queued frame. Present mode is a KNOB; the backend is
  isolated so a real Mailbox (go-webgpu) is a swap, not a rewrite. Pick ONE
  pacer — the blocking swap OR the ticker, never both timing the same frame.
- Loop: pump events every tick, then `select { case g := <-grids: latest = g;
  case <-tick.C: present(latest) }` → render `latest`, alloc-free.

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
  render.go        Renderer interface (Pump / Present / Close)
  atlas/           the shared glyph atlas (both renderers sample it)
  soft/            pure-Go software renderer (reference + regression oracle)
  gpu/             go-gl backend (//go:build cgo + a non-cgo stub; needs a desktop to run pixels)
  glass.go         the loop (newest-Grid → render)
  cmd/glass/       the desktop binary (composition root)
  soft/testdata/*.png   golden images
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

## Known v1 deviations (honest limitations — stated, not hidden)

The reference is a *fixed-size v1 viewer*. It is cited for the render
architecture (the seam, the loop, the parity harness), NOT as proof of resize,
pacing, HiDPI text, or accessibility. What v1 does not yet do:

- **Per-frame autoscale strobe.** `bases()` normalizes the ramp to the current
  frame's max, so one large order landing/cancelling re-brightens every cell at
  once. rsx-term uses a slow-decaying stable basis to stop the strobe; carrying
  that reference state across frames is the v2 fix. The stable-scale RULE cites
  rsx-term's behaviour, not glass's.
- **Hue-only trade side — FIXED (2026-07-19), upstream then mirrored.** Side
  is now a shape: mirrored per-side ramps `▵▴△▲` (buy, up = lifts) / `▿▾▽▼`
  (sell, down = hits) with the aggressor hue as the redundant channel, and the
  trade-overlay background capped at tier 1 so both inks clear 3:1 (uncapped, a
  buy print on a tier-4 wall was fg==bg). Fixed in the language
  (`rsx-term/VISUALS.md`, `notes/trade-side.md`), mirrored here (`heat.go`,
  `atlas/`), gated by `TestGlyphContrast` + `TestTradeSideByShape`.
- **Muted ink is APCA-sub-floor.** `#586b62` on the page is WCAG 3.54:1
  (passes 3:1) but APCA Lc ≈ 23, under the Lc 30 any-text floor — it carries
  the gutter time labels and `~mark`. Debt, not a local fix: the hex is
  verbatim-shared with rsx-term, the dashboard, and spec 55; a bump must be a
  coordinated palette rev (tracked in `rsx-term/VISUALS.md` "Known design
  debt").
- **Fixed 14 pt / 1× atlas, no HiDPI text.** The letterbox scales the grid by an
  integer factor with NEAREST — crisp-chunky by design, not re-hinted. Size-
  bucketed atlases rebuilt from content scale are v2.
- **Cell-grid resolution ceiling.** v1 renders the terminal's 72×24 picture as
  constant-colour blocks. For the continuous price×time heat *field* the GPU's
  real payoff is sub-cell resolution (per-datum instances or a data texture);
  primitive-follows-data-topology is v2.
- **sRGB is a fragile, deliberate invariant.** Parity holds ONLY because both
  renderers blend in non-linear sRGB and NO sRGB framebuffer is used. Do not
  enable `GL_FRAMEBUFFER_SRGB` or allocate an sRGB FBO without also linearizing
  soft — the GPU would silently linearize, parity would break, goldens would not.
- **go-webgpu swap is designed-for, unproven.** The seam has never held a second
  *windowed* backend — window creation, the swap-interval knob, and event
  pumping still live inside `gpu/`, so a webgpu backend must replace more than
  `soft` proves. The maintained binding is
  [cogentcore/webgpu](https://github.com/cogentcore/webgpu); the older
  rajveermalviya/go-webgpu is archived.
- **Wayland/XWayland.** go-gl/glfw builds the X11 backend by default; on a
  Wayland-default desktop the app runs under XWayland, where fractional scaling
  can blur the 1:1 pixel text. A `wayland` build tag exists.
- **Mid-session staleness.** If the feed dies mid-session the model keeps its
  last state and the render keeps painting it; only the header link dot flips.
  Freshness at the point of reading (mid dims / gets `~` after N ms) needs clock
  plumbing through `Scene` and is v2.
- **No accessibility surface.** Raw GL draws pixels: there is no accessibility
  tree, focus, screen-reader text, selection, or IME. This is a canvas; an
  accessible table representation would be a separate layer.

## Out of scope (v1 — keep it minimal)

- **Input / trading.** v1 is a read-only viewer (the heatmap; Esc/q quits).
  Keyboard order-entry is v2 (reuse rsx-term's keymap then).
- **Mailbox / go-webgpu.** The seam allows it later; v1 is go-gl vsync-on.
- **240 Hz tuning.** The seams are ready; not turned on.
