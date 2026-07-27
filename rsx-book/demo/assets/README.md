# book demo — collage-intro assets

The versus intro composites four original character portraits into an
animated scoreboard, then hard-cuts to the terminal benchmark.

## Character slots

The tracked PNGs are replaceable. Keep these names; PNG, JPG, and WebP inputs
are cover-cropped to square:

| file | lane | impl | verdict |
|---|---|---|---|
| `neanderthal.png` | neanderthal | BTreeMap | cancel 5.5–10× slower at depth |
| `engineer.png` | the engineer | flat array | better, still scans |
| `genius.png` | the genius | slab arena | faster |
| `gigachad.png` | GIGACHAD | slab + Δ-compression | p50/p99 fixed-work winner ★ |

Missing files render as labeled placeholder boxes.

## Build

```
uv run gen_collage.py   # images -> frames/  (PNG sequence)
bash build.sh           # frames + ../book-live-opt.gif -> book-versus.{mp4,gif}
```

- `gen_collage.py` — motion + composition (Pillow). Edit `LANES` to retune bar
  fractions / labels.
- `build.sh` — ffmpeg cut (collage intro → terminal bench) + palette GIF.
- `book-versus.gif` is the tracked playground asset.

`frames/` and `*.mp4` are build intermediates.
