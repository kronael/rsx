# CLAUDE.md — rsx-glass

Local to `rsx-glass/`. Inherits the repo-root `../CLAUDE.md`. The native
desktop GPU renderer for the **rsx-term visual language** — the log-time
liquidity heatmap drawn as pixels instead of terminal cells. All Go; the
GPU/native code is concentrated behind ONE interface so it stays movable.

## rsx-term is the template — ALWAYS

This crate exists to put rsx-term's picture on the GPU, nothing more. When in
doubt, **do what rsx-term does**:

- **Its visual design system is law here.** `../rsx-term/VISUALS.md` (the
  language — palette, glyph layers, the three compressions with exact
  boundaries, ramp math), `../rsx-term/SCREENS.md` (per-screen element
  inventory), `../rsx-term/notes/element-inventory.md` (the prune list). The
  `soft` renderer reproduces that spec pixel-for-pixel; the golden is the proof.
- **Prune to rsx-term's element set.** Don't add a glyph, colour, or region
  that rsx-term doesn't have — and lean toward the "cuttable/mergeable" tags in
  its element-inventory.
- **Its honesty rules carry over.** `—` unknown, `~` estimate, aggregate rows
  labelled, hard-block over the cap. The renderer draws what the model gives; it
  never fabricates.
- **Its minimalism carries over.** rsx-term is deliberately small; so is this.
  Resist scope creep.

## Keeper invariants — do NOT regress

- **The movable seam.** All GPU/native code lives behind the `Renderer`
  interface (`render.go`); the app depends on that interface, **never** imports
  `go-gl`/`go-webgpu` directly. Swapping OpenGL → go-webgpu (Mailbox) is one
  package. A `grep 'go-gl\|glfw\|webgpu' *.go` outside `gpu/` must be empty.
- **The render is a pure function of the newest `Grid`.** No retained scene
  graph. `Run` (`glass.go`) coalesces a burst of grids to the latest — never a
  backlog of stale frames.
- **Data-rate ⟂ render-rate.** Render on a tick off the newest grid; present
  mode (vsync-on now, Mailbox later) is a knob inside the backend, not the loop.
- **Alloc-free per frame** after warm-up (RSX hot-path discipline → the GC never
  runs on the render loop). Enforced by an allocs==0 test.
- **Reuse, don't re-implement.** The model (book folds, heatmap binning,
  fisheye) is rsx-term's; this crate imports it (`replace rsx-term => ../rsx-term`)
  and consumes its cell grid at the model→render seam. Any rsx-term edit keeps
  its DOM/book goldens green.
- **The `soft` golden is byte-locked** (rsx-term's golden discipline): `soft`'s
  reference PNGs must stay stable; a regression is a failing test, not a re-bless
  without review.

## Layout

| File | Role |
|---|---|
| `grid.go` | `Cell`/`Grid` — the plain-data contract between model and renderer |
| `render.go` | the `Renderer` interface — the movable seam (Present/Close) |
| `glass.go` | `Run` — the newest-grid render loop (coalescing, alloc-free) |
| `heat.go` | builds a `Grid` from the book/heatmap model (the rsx-term reuse) |
| `demo.go` | a synthetic `Grid` for goldens/examples (no cluster needed) |
| `atlas/` | the glyph atlas (rasterize the rsx-term glyph set once) |
| `soft/` | the pure-Go software renderer — reference, test oracle, headless, fallback |
| `gpu/` | the go-gl backend (build-tagged; the desktop path, behind `Renderer`) |

## Testing (the contract-verified-mock architecture)

- **Golden-image tests** (`soft/testdata/*.png`) lock the `Grid → pixels`
  contract, headless, no GPU.
- **Integration against a mock Go exchange.** The full data path
  (exchange wire → model → `Grid` → `soft`) runs against a **mock Go server that
  behaves like the RSX gateway/marketdata**, so tests are fast + deterministic +
  need no cluster. The integration suite covers data flows and wire formats.
- **The mock is kept honest.** A small contract slice reverifies the mock's
  formats/flows against the REAL exchange, so the mock can't drift from reality.
- Everything above runs automated + headless.

## When you touch this crate

- New backend → a package under a build tag implementing `Renderer`; the app
  imports it only at the composition root, never in library code.
- New visual → first check it exists in rsx-term (this is a *renderer*, not a
  place to invent visuals); update the golden with review.
- Run `gofmt`, `go vet`, `go test ./...` (goldens + allocs==0), and rsx-term's
  goldens if you touched its seam, before committing.
