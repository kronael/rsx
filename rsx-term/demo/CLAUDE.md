# demo/rsx-term — the terminal walkthrough

The rendered demo of the rsx-term trading terminal: a driven walk through
its screens on the **live RSX cluster**, filmed to a GIF for the READMEs
(root + this crate). Inherits the repo-root `../../CLAUDE.md`.

Unlike the other crate demos (speed pitches via `pitch.py` / `bench-live.sh`),
this is a **functional walkthrough** — it drives the real TUI and records what
a user actually sees, no narration. The keycast overlay (`RSX_TERM_KEYCAST=1`,
see `../notes/keycast.md`) shows every keystroke as a bottom-right chip
labelled from the keymap, so the viewer sees the keys as well as the effect.

## What the walk shows (`walk.sh`)

Boots the terminal on the live RSX trio (`make demo` from the repo root:
PENGU/SOL/BTC quoting, trading enabled as seeded user 1) with Hyperliquid
riding read-only, then:

1. **BOOK heatmap** streams — real maker/taker fills paint the trade layer.
2. **Price cursor** — `h`/`l` one tick, `j`/`k` snap to the bid/ask touch.
3. **Microscope** — `↑` scrubs older rows (honest aggregate labels); `enter`
   freezes the row → assistant (the reply cooks in the background).
4. **Trading** — `b` side, `f` resting limit at the cursor (◇ rests), `d`
   cancel; `r`/`p` mode toggles; `x` + code symbol hop PENGU → SOL → home.
5. **NEWS** — sector tiles + cross-symbol co-movement + `/` search.
6. **LLM** — the frozen handoff's REAL streamed arizuko reply, then a typed
   question → a second real reply (the payoff — a genuine round-trip).
7. **VENUES** — `F9` picker → hyperliquid (real BTC book, read-only) → back;
   `?` keymap-generated help; `q`.

No IOC cross in the tour: the demo cluster's completion path wedges minutes
after boot (BUGS.md `RISK-CMP-DESYNC-ORDER-EVENT-STARVE`); place/cancel is
the reliable window. Re-add `k '!'` once that's fixed.

## How it's rendered

`walk.sh` drives the binary in a headless tmux pane (status bar hidden so the
recording is just the terminal). `asciinema` records the pane, `agg` renders
the cast to a GIF, `gifsicle -O3` optimizes it:

    make rec    # build binary + record → term-live.cast  (needs the cluster live)
    make gif    # term-live.cast → term-live.gif + term-live-opt.gif
    make clean

- Bring the cluster up FRESH first: `./rsx-playground/playground reset --yes
  && make demo` from the repo root, then record immediately — the demo
  cluster's order path and md coherence degrade minutes after boot (see
  BUGS.md `RISK-CMP-DESYNC-ORDER-EVENT-STARVE` /
  `ME-MAKER-ORPHANED-QUOTES-WEDGE-BOOK`). Check the terminal's touch ladder
  is uncrossed before recording; reboot the cluster if it isn't.
- `RSX_TERM_ASSIST` (a live arizuko `/chat/{token}` URL) makes the assistant
  round-trip real; `make rec` best-effort mints one via
  `.ship/45-ARIZUKO-LLM/chat-token.sh`. Unset, the pane honestly shows the
  offline placeholder.
- 118×33, github-dark, font 14 — keep these in sync between `walk.sh`'s tmux
  size and the `agg` flags, or the GIF crops.
- If the cast ends on the dead pane's `[exited]` shell noise, trim the events
  after the final app frame (~the `q` press) before `make gif` so the GIF
  holds on the terminal, not a blank shell.

## Artifacts

- `walk.sh` — the driver (checked in; the recording is reproducible).
- `term-live.cast` — the asciinema recording.
- `term-live.gif` / `term-live-opt.gif` — rendered (raw / gifsicle-optimized).
  `term-live-opt.gif` is what the READMEs embed.
