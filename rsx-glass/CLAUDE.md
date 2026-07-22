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

## Design system — how the renderer consumes the visual language

The visual LANGUAGE is rsx-term's (`../rsx-term/VISUALS.md`); the rendering-side
DISCIPLINE is the `go-gl` skill + `~/.claude/skills/go-gl/design-systems-research.md`
(Material 3 / Fluent / Carbon / DTCG tokens, WCAG+APCA). Crate-local rules that
are load-bearing here:

- **Three token tiers.** primitive (ramp stops, spacing/type scales, the
  Okabe-Ito sequence) → semantic (`ColorLive`/`ColorAsk`/… by MEANING + reserved
  `OnRamp` glyph inks) → component (`cell.bg`, `cell.glyph.fg = OnRamp`). The
  render reads only semantic/component tiers; `UseTheme` rebinds tier 2
  (bid/ask) only.
- **Reserved-contrast inks, gated.** Every glyph fg is a palette ink ≥3:1 (and an
  APCA Lc floor) on every bg it sits on — NEVER "bg one tier up" (that was the
  fg==bg invisible-wall bug). `TestGlyphContrast` (`heat_test.go`) asserts the
  tier×ink matrix.
- **Meaning = colour + a VISIBLE redundant channel** (glyph/shape/position);
  hue carries CATEGORY only, never a meaning by hue alone.

Keep RSX's strengths (colour=meaning, glyph-as-shape channel, side=position AND
side-by-shape trade ramps `▵▴△▲`/`▿▾▽▼`, the `UseTheme` rebind seam, honesty
markers) — already ahead of generic HIGs. The remaining v1 deviation (Muted's
APCA debt) gets fixed upstream in the visual language, not forked here — see
`SPEC.md` "Known v1 deviations". The autoscale strobe is now fixed via the
shared `book.FoldBasis` stable basis (rsx-core), folded per bin by the model.

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
  fisheye) is rsx-term's, packaged as `rsx-core`; this crate imports it
  (`replace rsx-core => ../rsx-core` in `go.mod`) and consumes its cell grid at
  the model→render seam. Any rsx-core edit keeps its DOM/book goldens green.
- **The `soft` golden is a REGRESSION oracle, byte-locked** (rsx-term's golden
  discipline): `soft`'s reference PNGs must stay stable; a drift is a failing
  test, not a re-bless without review. It pins that the render did not *change*,
  not that the picture is *correct* — soft and gpu share the atlas and cell
  assumptions, so parity can only catch divergence between them, never a shared
  wrong encoding.

## Layout

| File | Role |
|---|---|
| `grid.go` | `Cell`/`Grid` — the plain-data contract between model and renderer |
| `render.go` | the `Renderer` interface — the movable seam (Pump/Present/Close) |
| `glass.go` | `Run` — the newest-grid render loop (always-pump events, coalesce to newest, alloc-free) |
| `heat.go` | builds a `Grid` from the book/heatmap model (the rsx-term reuse) |
| `feed.go` | the feed adapter — folds decoded `wire.*` events into the reused model → `Scene` |
| `demo.go` | a synthetic `Grid` for goldens/examples (no cluster needed) |
| `atlas/` | the glyph atlas (rasterize the rsx-term glyph set once) |
| `soft/` | the pure-Go software renderer — reference, test oracle, headless, fallback |
| `live/` | the market-data connector — WS subscribe + rsx-term's `wire.DecodeMd` → normalized events |
| `mockx/` | the mock RSX exchange for tests (real wire encoder + scripted scenario + WS server) |
| `gpu/` | the go-gl backend: instanced-quad cell grid + glyph-atlas texture, GLFW window / FBO context (the desktop path, behind `Renderer`; the ONLY package that imports go-gl). `//go:build cgo`; `stub.go` (`!cgo`) keeps `CGO_ENABLED=0` builds green and the parity test skipping cleanly |
| `cmd/glass/` | the desktop binary — the composition root that wires the feed → `gpu.New` → `glass.Run` (the one place `gpu` is imported) |

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
- **Renderer parity** (`gpu/parity_test.go`): the `gpu` backend renders the same
  `Grid` off-screen (GLFW hidden window + FBO readback) and must match the `soft`
  reference within colour rounding (float vs integer sRGB → maxDiff 1 level). It
  needs a GL context, so it **skips cleanly** with no display and runs on a
  desktop or under `xvfb-run go test ./gpu/`. It never fakes a pass.

## When you touch this crate

- New backend → a package under a build tag implementing `Renderer`; the app
  imports it only at the composition root, never in library code.
- New visual → first check it exists in rsx-term (this is a *renderer*, not a
  place to invent visuals); update the golden with review.
- Run `gofmt`, `go vet`, `go test ./...` (goldens + allocs==0), confirm
  `CGO_ENABLED=0 go build ./...` stays green (the soft path + gpu stub), and run
  rsx-core's goldens if you touched its seam, before committing. The GPU pixels
  are pixel-verified with `xvfb-run go test ./gpu/`.
