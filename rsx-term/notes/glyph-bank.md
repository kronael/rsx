# Why the heatmap glyphs are measured, not guessed

The heatmap paints every price×time cell with one character. That character —
together with its colour — has to carry *how much* liquidity rests there (size),
*how many* orders make it up (count), and whether a trade printed. So the first
question the renderer has to answer is mundane but load-bearing: **which
characters, in what order, give a perceptually even intensity ramp?**

The obvious answers are both wrong in ways you only discover by measuring the
pixels the font actually draws.

## Problem 1 — the shade blocks are not evenly spaced

The reflex is to reach for the Unicode shade family and assume it's a linear
ramp: space, `░` (light shade), `▒` (medium), `▓` (dark), `█` (full) = 0, 25,
50, 75, 100 % ink. Build a size→shade ramp on that assumption and the low end
bands: two very different depths both round to `░`.

Rasterised in the terminal's actual font (DejaVuSansMono) and measured by
black-pixel fraction, the real coverages are:

```
' ' 0.00   ░ 0.22   ▒ 0.56   ▓ 0.86   █ 1.00
```

The steps are 0.22, **0.34**, 0.30, 0.14 — not 0.25 each. The gap from `░` to
`▒` is more than 1.5× the gap from `▓` to `█`. A ramp that treats them as equal
puts most of its resolution in the wrong place.

## Problem 2 — braille is invisible

The seductive next idea is braille (`U+2800`–`U+28FF`): a 2×4 dot grid promises
**eight sub-cell "pixels" per character**, i.e. a 9-level density ramp finer than
the four shade blocks, and the sub-cell geometry could even imply direction.

Measured, every one of the 256 braille codepoints renders as an *identical*
0.23-coverage box. DejaVuSansMono ships no braille glyphs, so each one falls back
to the same `.notdef` tofu rectangle. A braille density ramp would render as a
column of identical empty boxes — worse than useless, because it *looks* like
it's working. (This isn't unique to one font: notcurses' own blitter docs warn
that poor font support "can ruin" the braille/quadrant blitters, and terminal
emulators have shipped braille cells with ~24 % font padding that break
continuity. Braille as a data channel is a font lottery.)

## The fix — build a glyph bank by rasterising and measuring

Stop assuming what a codepoint *promises* and measure what the font *renders*.
For every candidate glyph, draw it into a cell-sized bitmap in the target
monospace font and record three things:

- **coverage** — the black-pixel fraction (its perceived darkness / intensity);
- **centroid** — where the ink sits (top/bottom, left/right — its directionality);
- **quadrant weights** — how evenly the ink fills the cell (uniform fill vs. a
  shape).

This is the same trick ASCII-art renderers (`chafa`, `jp2a`) use to build their
glyph→luminance ramp; here it's pointed at a *trading* heatmap instead of a
photo. The measured bank then answers the ordering question empirically, and — a
free bonus — sorts glyphs into the roles they're actually good at:

- **Shades** `░▒▓█` are the only *uniform full-cell* fills, but there are just
  four coarse levels (above). → colour carries the *fine* intensity gradation;
  shade carries the *coarse* texture.
- **Letters and punctuation** fill the light end *finely* (coverage ≈ 0.07–0.35)
  and — unlike braille — render in *every* font. Ordered by coverage they give
  the classic ASCII-art ramp; combined with the shade blocks the result is fine
  everywhere and universally renderable: ` ~ ( 2 6 N ▒ ▓ █`.
- **Eighth-blocks** `▁▂▃▄▅▆▇█` measure as a clean, even 8-step ladder (0.13 →
  1.00 in ~0.13 steps) — but their ink is bottom-weighted, so they're *directional
  bars*, not uniform fill. → use them for depth bars, never for cell intensity.
- **Markers** `●◆■○` are distinct *shapes*, not densities. → they carry the
  *trade* layer and special states (your resting orders, long-standing
  liquidity) as a glyph channel visually separate from the shade/colour that
  encode resting size.
- **Braille** is dropped entirely (tofu).

## The cost it removes

Three failure modes, all invisible until they bite a user mid-trade:

1. **Banding** — an assumed-linear ramp wastes resolution where the eye needs it;
   a coverage-ordered ramp doesn't.
2. **Tofu** — a font-verified vocabulary can't ship a "finer" channel that
   renders as empty boxes on the user's terminal.
3. **Overloaded channels** — measuring separates *uniform-fill* glyphs (intensity)
   from *directional* glyphs (bars) from *shapes* (markers), so size, depth, and
   trades each get a channel that reads distinctly instead of fighting.

## The through-line

**A character is a picture; treat its ink as data.** Every decision here comes
from measuring the rendered pixels rather than trusting the codepoint's name —
which is also why the calibration is honest about its own limits.

## Caveat — calibration is per-font

The numbers above are DejaVuSansMono. Coverages and, crucially, *which glyphs are
even available* (braille, octants — the latter only entered Unicode 16 in 2024
and most monospace fonts still don't ship them) change per terminal font. The
glyph bank is therefore a *re-runnable tool*, not a frozen table: point it at the
font your terminal actually uses and it re-measures. The renderer also degrades
truecolour → 16-colour → plain, so the shade/letter fallback still reads when
colour is gone.

---

*Prior art: the glyph→luminance mapping is standard in ASCII-art renderers
(`chafa`, `jp2a`); the font-coverage caveat is documented in notcurses' blitter
notes and terminal-emulator braille-padding bugs. The measurement tool lives in
`tools/glyphbank` (rasterise → measure coverage/centroid/quadrants → emit the
calibrated ramp + a labelled contact sheet).*
