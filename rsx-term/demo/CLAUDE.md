# demo/rsx-term — the terminal walkthrough

The rendered demo of the rsx-term trading terminal: a driven walk through
its screens on a **live venue**, filmed to a GIF for the READMEs (root +
this crate). Inherits the repo-root `../../CLAUDE.md`.

Unlike the other crate demos (speed pitches via `pitch.py` / `bench-live.sh`),
this is a **functional walkthrough** — it drives the real TUI and records what
a user actually sees, no narration.

## What the walk shows (`walk.sh`)

Boots the terminal on `phoenix.trade` (real Solana-perp books, no local
cluster needed), then:

1. **BOOK heatmap** streams — the log-time liquidity heatmap (a text Bookmap).
2. **Microscope** — `↑` scrubs the row-cursor up the history; the mode line
   shows the exact ~100 ms bin.
3. **Freeze → assistant** — `enter` freezes the window and hands it to the LLM
   pane.
4. **NEWS** — `tab` to the cross-symbol co-movement overview.
5. **Help** — `?` shows the keymap-generated help; `esc`, then `q` to quit.

## How it's rendered

`walk.sh` drives the binary in a headless tmux pane (status bar hidden so the
recording is just the terminal). `asciinema` records the pane, `agg` renders
the cast to a GIF, `gifsicle -O3` optimizes it:

    make rec    # build binary + record → term-live.cast  (needs the venue reachable)
    make gif    # term-live.cast → term-live.gif + term-live-opt.gif
    make clean

- `RT_VENUE=rsx make rec` records against the RSX 3-token book instead — bring
  the cluster up first (`make demo` from the repo root).
- 118×33, github-dark, font 14 — keep these in sync between `walk.sh`'s tmux
  size and the `agg` flags, or the GIF crops.

## Artifacts

- `walk.sh` — the driver (checked in; the recording is reproducible).
- `term-live.cast` — the asciinema recording.
- `term-live.gif` / `term-live-opt.gif` — rendered (raw / gifsicle-optimized).
  `term-live-opt.gif` is what the READMEs embed.
