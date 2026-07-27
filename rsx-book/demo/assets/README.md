# book demo — collage-intro assets

The versus/collage intro composites **4 character images you provide** into an
animated scoreboard, then hard-cuts to the real terminal bench.

## Drop your images here

Name them exactly (png / jpg / webp — auto cover-cropped to square):

| file | lane | impl | verdict |
|---|---|---|---|
| `neanderthal.png` | neanderthal | BTreeMap | 1.5–2× slower, grows with depth |
| `engineer.png` | the engineer | flat array | better, still scans |
| `genius.png` | the genius | slab arena | faster |
| `gigachad.png` | GIGACHAD | slab + Δ-compression | ~70 ns flat — wins ★ |

Sourcing/licensing the images is the maintainer's call — they are not fetched
automatically (a build agent won't download third-party/copyrighted art into a
public promo). Missing files render as labeled placeholder boxes.

## Build

```
uv run gen_collage.py   # images -> frames/  (PNG sequence)
bash build.sh           # frames + ../book-live-opt.gif -> book-versus.{mp4,gif}
```

- `gen_collage.py` — motion + composition (Pillow). Edit `LANES` to retune bar
  fractions / labels.
- `build.sh` — ffmpeg cut (collage intro → terminal bench) + palette GIF.
- Output `book-versus.gif` is the postable; wire it into the `/demos` gallery.

`frames/`, `*.mp4`, and generated GIFs are build intermediates.
