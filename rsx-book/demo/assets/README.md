# book demo — collage-intro assets

The demo composites four deliberately different meme archetypes into an
animated scoreboard, then ends on the measured p50/p99 verdict.

## Character slots

The tracked PNGs are replaceable. Keep these names; PNG, JPG, and WebP inputs
are cover-cropped to square:

| file | lane | impl | verdict |
|---|---|---|---|
| `neanderthal.png` | neanderthal | ordered-tree baseline | starting point |
| `engineer.png` | the engineer | + direct level table | direct price access |
| `genius.png` | the genius | + slab handles | direct cancel |
| `gigachad.png` | GIGACHAD | + Δ-compression | bounded levels + occupancy |

Missing files render as labeled placeholder boxes.

## Image provenance

- `neanderthal.png` — original BAYC-like gorilla generated for RSX; it does
  not copy a specific token.
- `engineer.png` — Professor Farnsworth character art sourced from the
  [Futurama Wiki](https://futurama.fandom.com/wiki/File:Farnsworth.png).
- `genius.png` — Galaxy Brain meme image sourced from
  [Meme Arsenal](https://www.meme-arsenal.com/en/create/template/12349749).
- `gigachad.png` — standard Gigachad meme portrait sourced from
  [Meme Arsenal](https://www.meme-arsenal.com/en/create/template/9722082);
  the underlying Sleek'N'Tears image is by Krista Sudmalis.

The latter three are third-party reference/meme images and are not relicensed
under the project's GPL-3.0-only license. See the root `NOTICE`.

## Build

```
uv run gen_collage.py   # images -> frames/  (PNG sequence)
bash build.sh           # frames -> book-versus.{mp4,gif}
```

- `gen_collage.py` — sequential medallions, typed design steps, and one
  persistent design-progression meter. The final card alone carries measured
  p50, standard error, and p99.
- `build.sh` — ffmpeg cut (collage intro → terminal bench) + palette GIF.
- `book-versus.gif` is the tracked, graphics-only playground asset.

`frames/` and `*.mp4` are build intermediates.
