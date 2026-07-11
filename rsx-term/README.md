# rsx-term

A keyboard-only, streaming **text Bookmap**: the whole order book *and its
recent history* as a live liquidity heatmap in your terminal — plus a
market-overview screen and an LLM context screen — over a generic multi-exchange
seam.

## A few words first (plain English)

- **Order book** — the live list of resting buy and sell orders at each price.
- **Liquidity heatmap** — a picture of that book *over time*: where liquidity
  rests, where it's pulled, where trades eat it. (The GUI original is Bookmap;
  this is the text-terminal take.)
- **Fisheye** — a price axis that's fine near the current price and compressed
  far from it, so you see both the touch and the deep walls in one width.
- **Tick / mid** — the smallest price increment; the midpoint between best bid
  and best ask.

## What it is

A single Go binary that connects to an exchange over WebSocket and renders, in a
character grid, a **multi-resolution heatmap** of the book: time flows up on a
log cadence (live → 10 s → minutes → hours), price runs on a fisheye axis, and
each cell's colour and glyph encode resting size and order-count. Trades overlay
as a second layer, so **spoofing and absorption become things you *see***, not
things you infer after the fact. It's keyboard-only — there is no mouse.

## Why it exists

The open-source TUI-finance genre is all "quote + candle chart"; a terminal
order-book depth visualiser essentially didn't exist. The GUI incumbents
(Bookmap, Jigsaw, Sierra Chart) are heavy, closed, single-box, and stack their
cost across platform + data + per-feature add-ons. And they concede the limit:
hidden-order / absorption signals are reactive and probabilistic by their own
docs. So the frontier isn't fancier detection math — it's **honest presentation,
fewer windows, keyboard speed, and being able to run anywhere over SSH.** That's
the gap this fills.

## What it gives you

- **The log-time liquidity heatmap** — the book and its history at a glance;
  walls appearing and pulling, trades hitting or being absorbed, all visible.
- **Three screens, one key apart** — BOOK (the heatmap, where you trade and can
  freeze a moment for the assistant), NEWS (sector map + feed + cross-symbol
  co-movement), LLM (a context handoff).
- **Keyboard game-entry** — size presets on `1`–`5`, aggressive on shift, one key
  to place / one to cancel, a price cursor on `h`/`l`, a persistent mode line.
- **Generic multi-exchange** — RSX (your exchange) for trading, read-only
  Hyperliquid for real breadth; a new venue is one file (see
  `notes/venue-seam.md`).
- **Self-documenting** — one keymap table drives dispatch, the hint line, and the
  generated `?` help; rebindable via `RSX_TERM_KEYMAP`.
- **Honest** — it never fabricates a number (see `notes/honesty.md`), it's
  offline by default, and it degrades truecolour → 16-colour → plain.

## Quick start

```sh
# See it now — offline mock feed, no cluster needed:
RSX_TERM_STREAM=1 make term-demo         # press ? for help, q to quit

# Against a live local RSX cluster (run `make local` first):
RSX_TERM_STREAM=1 make term-local

# As a standalone Hyperliquid terminal (real books, read-only):
cd rsx-term && RSX_TERM_STREAM=1 RSX_TERM_VENUE=hyperliquid go run .

# Or a standalone phoenix.trade terminal (Solana perp DEX, read-only):
cd rsx-term && RSX_TERM_STREAM=1 RSX_TERM_VENUE=phoenix go run .
```

`RSX_TERM_VENUE` selects the market-data venue(s): `rsx` (default) ·
`hyperliquid` · `phoenix` (standalone read-only over one venue's feed) ·
`both` (RSX + HL) · `all` (RSX + HL + Phoenix) · or a comma list
(`rsx,phoenix`). `RSX_TERM_HL_COINS` / `RSX_TERM_PHX_SYMBOLS` pick the watch set
(`all` = every market).

`RSX_TERM_NEWS=1` enables the Tree of Alpha news reader;
`RSX_TERM_ASSIST=http://127.0.0.1:8095/chat/<token>` wires the LLM pane to a
locally deployed arizuko agent (the full chat URL incl. the minted route token —
unset keeps the offline placeholder and dials nothing);
`RSX_TERM_COLOR=plain|shade|true` forces the render mode. Everything is
env-configured; nothing dials the network unless you opt in.

## Guarantees

- **Keyboard-only.** No mouse input at all.
- **Offline by default.** The mock/default path makes zero network calls.
- **Never fabricates.** Unknown values dash; estimates are marked `~`; an order
  over the fat-finger cap is hard-blocked; an overflowing P&L is withheld, not
  wrapped.
- **The classic DOM view is byte-locked** by a golden test — the streaming work
  can't regress it.

## When NOT to use this

- **Not for automated / HFT trading.** At the exchange's µs latencies a human
  can't compete; that's what the API is for. This is a *discretionary manual*
  surface (and a demo / reference client).
- **Not a charting package.** No candles, indicators, or drawing tools — it shows
  order flow, not TA.
- **Not a Bloomberg replacement.** It's deliberately three focused screens, not
  an everything-terminal.

## Requirements

Go 1.26+. A truecolour terminal gets the full heatmap palette; it degrades
cleanly to 16-colour and plain. DejaVuSansMono (or a font with the block-element
glyphs) is assumed by the calibrated glyph bank — re-run `tools/glyphbank` for a
different font.

## How to read this crate

- **README.md** (this) — what it is, why, how to run it.
- **ARCHITECTURE.md** — how it's built: modules, data flow, the heatmap
  algorithm, invariants, and the decisions (with the alternatives rejected).
- **notes/** — *why* each design choice: `compression.md`, `glyph-bank.md`,
  `honesty.md`, `venue-seam.md`.
- **SCREENS.md / VISUALS.md / FLOWS.md** — what each screen shows, the visual
  language, and the end-to-end user journeys.
