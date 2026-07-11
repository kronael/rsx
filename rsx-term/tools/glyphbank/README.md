# glyphbank

Rasterises candidate terminal glyphs, measures ink coverage / centroid /
quadrant weights, and emits a coverage-ordered intensity ramp + a labelled
contact sheet. Calibrates the heatmap glyph vocabulary empirically per font.
See `../../notes/glyph-bank.md` for the why.

Run: `python3 glyphbank.py` (needs PIL + a monospace TTF; DejaVuSansMono by default).
