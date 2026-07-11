# Demo & first read — learn to see order flow in five minutes

This is a guided first run. It doubles as a short lesson: by the end you'll be
able to read a liquidity heatmap and spot the two patterns order-flow traders
live for. No account, no cluster, no network needed.

## 0. Run it

```sh
RSX_TERM_STREAM=1 make term-demo      # press ? any time; q to quit
```

You're now watching a scripted book stream. Nothing here touches the network.

## 1. Read the picture

The screen is a **heatmap of the order book over time**:

- **Up = time, on a log cadence.** The bottom row is *now* (live, ~100 ms). As you
  go up, each row covers an exponentially longer window — 10 s, a minute, minutes,
  hours. So the bottom is fine-grained and the top is the deep past, all on one
  screen. The right-hand gutter labels the rows (`−10s`, `−1m`, …). Recent rows
  are exact; the dim far rows are *summaries* of a window, and they say so.
- **Left/right = price, on a fisheye.** The centre gap is the spread (bids left,
  asks right). Near the touch it's one price-tick per column; far from it, columns
  aggregate many ticks — so you see both the fine near-touch structure *and* the
  deep walls in one width.
- **Each cell** is coloured by *how much* rests there (a heat ramp) and its glyph
  by *how many orders* make it up — so one whale reads differently from a stack of
  small orders. (Why the glyphs are chosen this way: `notes/glyph-bank.md`.)

## 2. The two patterns to look for

This is the whole point — things a static ladder *can't* show, because they live
in the time axis:

- **Spoofing.** A big wall appears a few ticks from the touch… and then *vanishes*
  as price approaches it, before any trade reaches it. On the heatmap that's a
  bright block that blooms and disappears across successive rows. Real support
  doesn't run away.
- **Absorption.** Trades (the bright `◆` marks) pound a resting level again and
  again and it *doesn't shrink*. Someone is quietly refilling — the level is
  absorbing the flow. That block stays lit while trades hammer it.

Watch a minute of the demo with those two shapes in mind. That's order-flow
reading.

## 3. Move around

- `tab` / `shift+tab` cycle the three screens: **BOOK** (this heatmap, where you'd
  trade), **NEWS** (a market overview — sector tiles, the news feed, which symbols
  move together), **LLM** (an assistant you can hand a frozen moment to).
- In BOOK: `1`–`5` arm order sizes, `h`/`l` move a price cursor, `f` places, `d`
  cancels, `x`+a letter code hops to another symbol. `?` shows the full,
  auto-generated key map. There is **no mouse** — every action is a keystroke.

## 4. Real markets

The same terminal runs against a live venue with real depth and flow:

```sh
cd rsx-term && RSX_TERM_STREAM=1 RSX_TERM_VENUE=hyperliquid go run .
```

Now the heatmap is Hyperliquid's real book (read-only — market data, no trading).
Everything you learned above applies; now the spoofs and absorption are real.

## 5. Notice the honesty

As you watch, note what the terminal *refuses* to fake: a value with no source
shows a dash `—`, not a made-up number; a client-side estimate is marked `~`;
the far, aggregated rows are dimmed and labelled as windows, never dressed up as
an exact book. It would rather show you a gap than a plausible lie. (Why:
`notes/honesty.md`.)

## The concepts, if you're learning

- **Order flow** — the stream of adds, cancels, and trades that *moves* price;
  the heatmap renders it directly instead of summarising it into a candle.
- **Spoofing / absorption** — the two readable intent-signals above; note that
  intent is never *provable* from book data, which is why the honest move is to
  show the pattern and let you judge.
- **The three compressions** (log-time, price-fisheye, log-size) that make a
  whole book + history fit a terminal — the maths is in `notes/compression.md`.

## Demoing to someone else (60 seconds)

Run `RSX_TERM_STREAM=1 make term-demo`, say "this is the order book *over time* —
watch this wall appear and then vanish before price gets there: that's a spoof,"
`tab` once to show the market overview, and quit with `q`. That's the pitch.
