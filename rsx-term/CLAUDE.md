# CLAUDE.md — rsx-term

Local to `rsx-term/`. Inherits the root `../CLAUDE.md`. This file pins the
**documentation conventions and the invariants that must not regress** — the
keeper list, so a later "cleanup" can't quietly gut what makes this crate good.

## Doc topology (which file answers which question)

Follow the `doc-topology` skill (itself distilled from rsx-cast/rsx-book):

| File | The one question | Notes |
|---|---|---|
| `README.md` | what / why / how-to-run | elevator pitch line 2, glossary before jargon, quick-start, guarantees, "when NOT to use", "how to read this" |
| `ARCHITECTURE.md` | how it's built | module table, data-flow diagram, the heatmap algorithm, invariants, decisions *with the rejected alternatives* |
| `ONEPAGER.md` | one-page distillation | keep it to a page |
| `DEMO.md` | guided first run that *teaches* order-flow reading | education-first |
| `notes/*.md` | why each design choice | Problem → Fix → Cost-it-removes, cited, one file per decision |
| `SCREENS.md` / `VISUALS.md` / `FLOWS.md` | what each screen shows / the visual language / user journeys | keep in lockstep with the code |

`notes/` is the *why* layer: `compression.md` (the three nonlinear
compressions), `glyph-bank.md` (empirical glyph calibration), `honesty.md`
(never fabricate), `venue-seam.md` (exchange = plugin), `assistant.md`
(chat pane → arizuko over a route token + SSE). Add one per new
non-obvious decision.

## Keeper invariants — do NOT regress

These are load-bearing; a "simplification" that drops one is a bug, not a cleanup:

- **Keyboard-only.** No mouse input, ever. The price cursor is `h`/`l`; there is
  no click-to-price.
- **Never fabricate a number.** Dash the unknown (`—`), mark estimates (`~`),
  hard-block over the fat-finger cap (never a dismissable warning), withhold an
  overflowing P&L rather than show it wrapped. See `notes/honesty.md`.
- **Offline by default.** The mock/default path makes zero network calls; every
  dial site sits behind an env opt-in (`RSX_TERM_VENUE`, `RSX_TERM_NEWS`) and a
  named goroutine.
- **The public feed is untrusted** — validate market-data frames at the `conn`
  edge (skip malformed sides/prices, don't coerce them).
- **The DOM view is byte-locked.** `TestDomViewGolden` / `TestBookViewGolden` /
  `TestDefaultViewUnchanged` must stay green; the streaming work can't regress the
  classic view or the book render.
- **The model is UI-agnostic.** `book/` folds are pure and clock-free; the
  render is a pure function of the model; the LLM handoff (`news.AssistantContext`)
  is a plain struct — so a future GPU/bitmap frontend replaces only the render.
- **Honest compression.** Far heatmap rows are aggregate windows, labelled as
  such — never dressed up as an exact book. No replay buffer, no lead-lag over
  jittery 100 ms mids (both deliberately rejected — see the direction pivot).

## When you touch this crate

- New design decision → a `notes/` file (Problem → Fix → Cost).
- New screen behaviour → update `SCREENS.md` + the keymap-generated help stays the
  source of truth for keys.
- Run `make fmt` + `go test -race ./...` + the goldens before committing.
- The glyph vocabulary is calibrated per font — re-run `tools/glyphbank` if the
  target terminal font changes.
