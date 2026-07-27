# Sourcing meme characters for the versus-collage demo

The book demo opens with a "who builds the fastest order book" scoreboard:
four characters race, then it cuts to the live cargo bench. You supply the
4 character images; the pipeline (`gen_collage.py` + `build.sh`) does the
rest. This file is **where + how to get them.**

> Why you do the download and not the build agent: embedding third-party
> meme *images* in a public repo is a copyright call only the maintainer can
> make. The agent won't fetch/curl copyrighted art in; it composes whatever
> you drop in. See `../../..` project memory `demo-memes-copyright-boundary`.

## The 4 slots

| file | archetype | book impl | verdict shown |
|---|---|---|---|
| `neanderthal.png` | dumb / caveman | BTreeMap | 1.5–2× slower, grows |
| `engineer.png` | nerd | flat array | better, still scans |
| `genius.png` | big-brain | slab arena | faster |
| `gigachad.png` | alpha / winner | slab + Δ-compress | ~70 ns flat — wins ★ |

Transparent-background PNGs look best (they composite onto the dark page).
The loader also accepts jpg/jpeg/webp and cover-crops to square.

## Where to download (transparent PNGs)

- **[pngimg.com/memes/wojak](https://pngimg.com/images/memes/wojak)** — Wojak/Feels-Guy, clean cut-outs
- **[pngall.com/wojak-png](https://www.pngall.com/wojak-png/)** — Wojak + Chad + brainlet variants
- **pngitem.com / pngfind.com** — search "brainlet", "galaxy brain", "chad", "gigachad"
- **[imgflip.com/memesearch](https://imgflip.com/memesearch)** — blank templates (incl. [transparent gigachad](https://imgflip.com/memetemplate/528615791/Transparent-gigachad)); 1M+ templates
- **[Know Your Meme](https://knowyourmeme.com/)** — canonical reference + image galleries per character
- **[ImgFlip575K dataset](https://github.com/schesa/ImgFlip575K_Dataset)** — bulk template dump if you want to script a pick

## Copyright status — pick with eyes open (public repo)

- **Pepe the Frog — avoid.** Clear owner (Matt Furie), [actively DMCAs users](https://knowyourmeme.com/memes/events/pepe-the-frog-dmca-takedowns). Real legal risk.
- **Gigachad** — copyrighted photograph of a real person (Ernest Khalimov). Widely used as a meme, but it *is* someone's copyrighted photo.
- **[Wojak family](https://en.wikipedia.org/wiki/Wojak)** (Wojak, Chad-drawing, brainlet, Soyjak, Doomer, NPC) — emerged anonymously on imageboards, **no clear owner, gray area** — the lowest-risk recognizable set. The drawn "Yes Chad" reads as the alpha just as well as the Gigachad photo.

**Recommended safe set:** brainlet-wojak (neanderthal) · soyjak/nerd-wojak
(engineer) · galaxy-brain-wojak (genius) · Yes-Chad drawing (gigachad).
All wojak-family, all gray-area, all on the PNG sites above.

## How to make / caption them

Templates rarely need captions here (the scoreboard supplies the text), but
if you want captioned faces:

- **[Kapwing meme maker](https://www.kapwing.com/meme-maker)** — upload/paste URL, add text, remove background, export PNG. Fastest.
- **Canva** — background remover is paid; else export PNG → Kapwing to cut out.
- **GIMP** ([tutorial](https://traneptora.com/articles/memetext/)) — Fuzzy-Select the background → Delete → export PNG. Free, local.
- **[imgflip generator](https://imgflip.com/memegenerator)** — pick template, type caption, Generate, download.

## Then: build it

```
cd rsx-book/demo/assets
# 1. drop neanderthal.png engineer.png genius.png gigachad.png here
uv run gen_collage.py     # images -> frames/
bash build.sh             # -> book-versus.{mp4,gif}
```

The gallery already points `rsx-book` at `assets/book-versus.gif`, so once
the file rebuilds, restart the dashboard (`./rsx-playground/playground stop
&& ./rsx-playground/playground start`) and `/demos` serves your real memes —
no code change. Retune bar lengths / labels in `gen_collage.py` `LANES`.
