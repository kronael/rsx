# Why trade side is a shape, not just a hue

## Problem

Prints on the heatmap (and the tape rail) drew ONE magnitude ramp `○◆●■`
coloured by aggressor: green = buy, red = sell. Hue was the *sole* carrier of
side — the textbook WCAG 1.4.1 failure. Under deuteranopia/protanopia (~8% of
men) the most important event on the screen lost its direction, and in
`modePlain` (colourless terminals) side vanished for *everyone*: the
degradation ladder's own floor dropped a meaning. A second, measured defect
hid in the same cells: the aggressor ink sat on the uncapped size-ramp
background, so a buy print on a tier-4 wall was `#22f5a1` on `#22f5a1` —
1.00:1, literally invisible (sell on tier 3: 1.09:1; own `◇` on tier 3:
1.00:1; the colourblind theme had the same collapse).

## Fix

Two mirrored per-side ramps, calibrated with the glyphbank method against
DejaVuSansMono like every other glyph family:

- `tradeBuyRamp` `▵▴△▲` — ink 0.079 / 0.110 / 0.144 / 0.263
- `tradeSellRamp` `▿▾▽▼` — ink 0.083 / 0.110 / 0.169 / 0.262

Up = the aggressor lifts the offer, down = hits the bid — the same polarity
the ruler's `▲`/`▼` own-order marks already taught, so the language stays one
rule: **up/down = buy/sell everywhere**. Hue remains as the redundant second
channel (colour + shape, per the redundant-channel rule).

The overlay background is capped at `overlayBgTier` = 1 (`overlayTier`), so
every overlay ink clears 3:1 on every background it can land on, in both
themes — measured worst pair after the cap: 3.91:1 / Lc 39 (colourblind bid
on tier 1). A print outranks the wall it lands on; the wall's exact tier is
still readable in the neighbouring rows the print doesn't cover.

`TestCellTradeSideByShape` asserts buy ≠ sell in every render mode including
`modePlain`; `TestPaletteContrast` gates the ink×bg matrix (the ANSI-stripped
view goldens can never see colour, so the matrix test is the eye).

## Cost it removes

- A colour-vision user (or any `modePlain` user) misreading — or simply not
  seeing — order-flow direction mid-trade.
- Invisible prints on walls: the highest-information cells (a trade hitting
  standing liquidity) were exactly the ones that collapsed to fg==bg.
- The old ramp's prettier ink range (0.16–0.51 vs 0.08–0.26) is the price:
  top-tier prints carry less ink than `■` did. Bold + hue + the capped
  (darker) background keep them salient; the trade-off buys a channel that
  survives desaturation.
