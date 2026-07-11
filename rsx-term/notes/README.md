# notes — why rsx-term is built the way it is

The design-rationale layer: one file per non-obvious decision (Problem → Fix →
Cost-it-removes). For *how* it is built see ARCHITECTURE.md; for *what you see*
see SCREENS.md; for *why*, here.

- **compression.md** — the three nonlinear compressions (log-time cadence, price
  fisheye, log-size colour) that fit a whole book + its history into a text grid.
- **glyph-bank.md** — why the heatmap glyphs are measured (rasterised + coverage-
  ranked), not guessed; why braille is excluded (tofu in the default font).
- **honesty.md** — why the terminal never fabricates a number: dash the unknown,
  label the estimate, hard-block not soft-warn, withhold on overflow.
- **venue-seam.md** — why an exchange is a plugin behind two tiny interfaces
  (Source + Submitter), not a fork; how RSX / Hyperliquid / mock all normalise
  to one wire model so the terminal is generic multi-exchange.
