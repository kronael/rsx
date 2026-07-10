# SCREENS — rsx-term

Every screen and state the terminal renders, with a mockup and what it means.
One screen, one symbol (PENGU-PERP); states are variations of it, not separate
pages. Rendering lives in `rsx-term/ui/view.go`; styling in `VISUALS.md`; the
step-by-step user journeys through these screens are in `FLOWS.md` (each backed
by a scenario test). Colours below are described by meaning (see the palette) —
a terminal shows them, this file can't.

## Main screen (live, position open)

The canonical layout: status bar, three columns, speed strip, status line,
help.

```
 RSX  PENGU-PERP   ● live  open 1  fills 7  last —  ~mark 10000 (mid)  index —  funding —
┌────────── book ──────────┐┌───────── order ─────────┐┌──── positions (mark=mid) ────┐
│      10006               ││  BUY      SELL           ││ sym  side  net  entry  ~uPnL │
│      10005               ││                          ││ PENGU-PERP  LONG  +14  … +140 │
│      10004  12           ││ price: _                 │└──────────────────────────────┘
│      10003               ││ qty  :                   │┌──────── orders ──────────────┐
│      10002  20           ││ time-in-force: GTC       ││ side  px  qty                │
│      10001   5           ││ reduce-only: off  po: off││▸ BUY   9999 15               │
│    7 10000               ││                          │└──────────────────────────────┘
│▸  15  9999               ││ enter → preview → send   │┌──────── trades ──────────────┐
│       9998               │└──────────────────────────┘│ B 10001 5                    │
│   30  9997               │                            │ S 10000 3                    │
│████████████ 58% bid      │                            └──────────────────────────────┘
└──────────────────────────┘
 ⚡ RTT 10.4 µs = net 2.5 µs + internal 7.6 µs + engine 340 ns  ██████████  p50 9.9 µs · p99 10.4 µs · best 9.0 µs  ▁▃█▅
 sent Buy 5 @ 10001 [GTC]
 q quit  b/s side  t tif  r ro  p po  +/- tick  j/k join  tab field  0-9 type  ⌫ del  enter submit  m mkt  ↑↓ sel  c cancel  X all  x flatten  R reverse  F2 armed  F3 trace  ? help
```

- **status bar** — violet symbol badge; green `● live` link dot; open/fills
  counts; `last`, `~mark N (mid)` in dim italic (a client estimate, not the
  exchange mark), `index —`, `funding —` (no source yet).
- **book — a static price ladder.** A fixed price axis centred on the mid
  (recentres only on drift, not every tick); **bid qty left / ask qty right**
  of the price column; empty prices are gaps, so the spread and thin
  liquidity read at a glance. Each level draws a **depth bar** (`▊`) scaled to
  the deepest visible level — bid depth grows left, ask right, so the bars
  converge on the spread (a DOM depth read). Your resting orders are marked
  `▸` on their rows, the last print `‹`; the price is coloured by the resting
  side. A bottom **imbalance bar** shows bid-vs-ask share of the visible depth.
- **order** — side toggle (active reversed), price/qty fields (focused one
  bold+bright with `_`), time-in-force, reduce-only/post-only, the two-step
  confirm hint. `+`/`-` nudge the price a tick, `j`/`k` join the best
  bid/ask, `m` sends a market IOC. `F2` arms **confirm-off** mode (a loud red
  banner; single-enter fire) — the fat-finger size guard still holds.
- **positions** — `mark=mid` flags the derived mark; stacked so the narrow
  column never wraps: `LONG` green / `SHORT` red + net `@` entry, then
  `~uPnL` (coloured, money at quote precision), then a dashed **risk row** —
  liq / ROE / margin-health — held honest (no fabricated number) until the
  risk feed lands.
- **orders** — your working orders (side/px/qty); the `▸` cursor marks the one
  `c` cancels (`↑↓` move it, `X` cancels all). Only shown when you have some.
- **trades** — newest first, `B`/`S` glyph + price/qty, coloured by side.
- **speed strip** — the ⚡ round-trip split net / internal / engine, a
  **proportional hop-bar** showing where the time goes, and rolling
  p50 / p99 / best + a sparkline. The RTT number goes amber/red on an SLA
  breach.

## Keys

`b`/`s` side · `0-9`/`⌫`/`tab` edit price+qty · `t` tif · `r`/`p`
reduce-only/post-only · `+`/`-` nudge price a tick · `j`/`k` join best
bid/ask · **left-click** a ladder row to set its price · `enter` preview →
`enter` send (`esc` cancels) · `m` market (IOC far touch) · `↑↓` select a
working order · `c` cancel it · `X` cancel all ·
`x` flatten (reduce-only) · `R` reverse the position · `F2` ARMED
(confirm-off) · `F3` telemetry trace · `?` help · `q` quit. Orders over the
fat-finger size cap are hard-blocked, never soft-warned.

## Startup — waiting for first round-trip

Before any order round-trip, the speed strip is dim:

```
 ⚡ latency: waiting for first round-trip…
```

## Order link down (offline)

The order gateway link dropped; the dot goes amber. Orders can't be sent
until it reconnects (auto-reconnect with backoff). Marketdata may still be
live — the two links are independent.

```
 RSX  PENGU-PERP   ● offline   open 0  fills 7   last 10001  ~mark 10000 (mid)  index —  funding —
```

## Degraded book — marketdata down or empty

When the marketdata link is down or the ladder is empty, the book panel
shows an amber message instead of a blank or fake ladder:

```
┌────────── book ──────────┐
│ no live book — market-data stream down │
└──────────────────────────┘
```

## Order entry — typing

`b`/`s` pick side (active side reversed), `tab` moves focus, digits and `.`
edit the focused field (bold + bright + `_` cursor), `t` cycles TIF, `r`/`p`
toggle reduce-only / post-only. You type the **human decimal you read off the
ladder** — `0.010001`, not the raw `10001`; it's reconstructed to the raw i64
wire value at submit:

```
┌───────── order ─────────┐
│   BUY      SELL          │   ← SELL reversed = active side
│                          │
│   price: 0.010001        │
│   qty  : 5_              │   ← focused field: bold, bright, cursor
│   time-in-force: IOC     │
│   reduce-only: on   post-only: off │
│                          │
│   enter → preview, enter again to send │
└──────────────────────────┘
```

## Confirm preview (submit guard)

First `enter` never submits — it renders a violet-headed preview inline in the
order panel (side, size, price, notional, TIF, flags). Second `enter` sends;
`esc` cancels. `liq` has no server source yet, shown as a deliberate `n/a`, not
a fake number.

```
┌─ (violet ring) ──────────┐
│ confirm BUY 5 @ 10001    │
│ notional 50005  GTC  ro:off po:off │
│ liq  n/a                 │
│ n/a fields need server support, not yet wired │
│ enter again to SEND · esc cancel │
└──────────────────────────┘
```

## Positions — the three states

Flat (no position):

```
┌──── positions (mark=mid) ────┐
│ sym  side  net  entry  ~uPnL │
│ no position — fills build it │
└──────────────────────────────┘
```

Open, priceable (a mid exists → uPnL green/red):

```
│ PENGU-PERP  SHORT  -20  10002  -40 │
```

Open, not priceable (no mid → amber caption, never a bare dash):

```
│ PENGU-PERP  LONG  +14  9998  —  (needs live book) │
```

## Trades — empty vs prints

```
┌──────── trades ──────────────┐        ┌──────── trades ──────────────┐
│ no trades yet                │   vs   │ B 10001 5                    │
└──────────────────────────────┘        │ S 10000 3                    │
                                         └──────────────────────────────┘
```

## Flatten (x)

`x` submits a **reduce-only** order that flattens the current net position
(net>0 → Sell |net|, net<0 → Buy |net|), through the same confirm preview so
it's not a fat-finger close. No-op with a status message when flat. Reduce-only
means it can only shrink, never flip, the position.

## Trace HUD (F3)

F3 swaps the right column for a violet-ringed diagnostics panel — endpoints,
both link states, rtt p50/min, open/fills, spread, depth, last event:

```
┌─ (violet ring) ──────────┐
│ TRACE — F3 to hide       │
│ endpoint ws://…:8080     │
│ md       ws://…:8180     │
│ link     connected       │
│ md link  connected       │
│ rtt p50  9.9 µs          │
│ rtt min  9.6 µs          │
│ open     2               │
│ fills    7               │
│ spread   1               │
│ depth    8 bid / 8 ask   │
│ last     sent Buy 5 …    │
└──────────────────────────┘
```

## Offline demo

`RSX_GW_URL=mock go run .` runs the whole terminal against a scripted feed
(`conn/mock.go` `DemoScript`) with no network — the reference screen for
this catalogue and for a sales demo. On the mock, all three latency legs are
real; live, only `net` is client-measured until the gateway stamps the
internal/engine legs (they show `·· pending`, dim italic — never a bare
dash, see `legValue`).

## Streaming heatmap (RSX_TERM_STREAM=1)

A prototype "text Bookmap": time runs **top → bottom**, one row per ~100ms
bin, **newest at the bottom**, older bins aging upward off a fixed-length
ring. Price runs **left → right** through a **mid-centred fisheye** — bids
left, asks right, the touch at 1 tick/cell, deep levels aggregating many
ticks into one column. The axis re-anchors on the mid only when it drifts
(hysteresis), so the picture doesn't reshuffle every tick. Rendering is in
`ui/stream.go`; the ring/fisheye/binning in `book/heatmap.go`. The classic
DOM view (above) is the default; this view is opt-in via `RSX_TERM_STREAM=1`.

**Each cell is two channels** (the shade ramp ` ░▒▒▓█` below stands in for
colour, which a terminal shows and this file can't):

- **Background colour = resting SIZE**, log-scaled (sizes are heavy-tailed):
  the side's hue, dim → saturated as size grows. Bid hue / ask hue are the
  palette's bid/ask pair (the colorblind theme swaps them for blue/orange).
- **Glyph (` ░▒▓█`) = resting ORDER COUNT**: one whale (huge size, 1 order)
  reads as a bright cell with a faint `░`; a wall of many small orders reads
  as a fuller `▓`/`█`. So "one big order" and "a crowd" look different even at
  the same size.
- **Trades** overlay a bright `◆` in the **aggressor's** hue at the trade
  column, brightest on the newest row and **decaying** over the next couple of
  rows as they age upward.
- Degrades: full two-channel RGB on 256/true-colour; **shades-only** (size via
  glyph, side via 16-colour hue) on a plain-colour terminal; a **colourless
  glyph ladder** when colour is unavailable (`RSX_TERM_COLOR=plain|shade|true`
  forces the tier; `NO_COLOR` is honoured).
- **Left rail** `│` is the **news axis** (placeholder): markers `►` line up
  with the bin they hit. The default source is off (`news.Off`), so offline the
  rail is a plain gutter — no network call is made at startup or on render. A
  live `news.TreeOfAlpha` stub documents `wss://news.treeofalpha.com/ws` but
  ships disabled.
- **Pinned footer** (does not scroll): exact live touch (best bid/ask px×size),
  position + `~uPnL` (mid-marked), latency (`⚡`), a one-line **LLM placeholder**
  (`? assistant …`), and the control legend.

**Idle** — a stable book: two resting clusters straddle the mid gap, the same
shape scrolling down bin after bin.

```
 RSX  PENGU-PERP   ● live  ◀ bids  mid 0.010000  asks ▶
│        ░░▒▒▓░░       ░░▒▓▓░░
│        ░░▒▒▓░░       ░░▒▓▓░░
│        ░░▒▒▓░░       ░░▒▓▓░░
│        ░░▒▒▓░░       ░░▒▓▓░░
│        ░░▒▒▓░░       ░░▒▓▓░░
bid 0.009999 × 0.0005   ask 0.010001 × 0.0006   spread 2
pos flat — fills build it
⚡ RTT 10.4 µs   p50 9.9 µs · p99 10.4 µs · best 9.0 µs
? assistant — context ready (placeholder)
 q quit  b/s side  +/- tick  enter submit  F3 trace  ? help  · streaming heatmap (RSX_TERM_STREAM)
```

**Wall building** — a large ask order stacks a tick above the touch (bright
background = big size; solid `█` because it's many orders — a real wall):

```
│        ░░▒▒▓░░     █▓░▒▓▓░░
│        ░░▒▒▓░░     █▓░▒▓▓░░
│        ░░▒▒▓░░     ██░▒▓▓░░
│        ░░▒▒▓░░     ██░▒▓▓░░   ← ask wall thickening into the touch
```

**Trade burst** — market buys lift the offer; bright `◆` marks print on the
newest rows at the trade column and decay upward:

```
│        ░░▒▒▓░░       ░░▒▓▓░░
│        ░░▒▒▓░░      ◆░░▒▓▓░░   ← older prints, dimmer
│        ░░▒▒▓░░     ◆◆░░▒▓▓░░
│        ░░▒▒▓░░    ◆◆◆░▒▓▓░░   ← newest bin, brightest
```

**Your fill** — after a fill the footer flips from flat to a live position and
mid-marked uPnL (the map itself is unchanged — your resting orders are a DOM
cue, deferred this prototype pass):

```
bid 0.009999 × 0.0005   ask 0.010001 × 0.0006   spread 2
pos LONG +0.0014 @ 0.009998   ~uPnL +0.000140
⚡ RTT 10.4 µs   p50 9.9 µs · p99 10.4 µs · best 9.0 µs
```
