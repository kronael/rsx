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
bid/ask · `enter` preview →
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

## Streaming terminal (RSX_TERM_STREAM=1) — four screens

Opt-in via `RSX_TERM_STREAM=1`; the classic DOM view above stays the default.
To promote streaming to the default, flip the `stream` check in `main.go` to
default-on (one line) — the founder decides that.

One keypress apart, four screens (`tab` cycles, `shift+tab` reverses):

```
 BOOK  ── depth, ONE symbol      quoting / market-making on the heatmap
 PAIR  ── breadth, MANY symbols  aggressive directional lots, no depth
 NEWS  ── market context         sector map + Tree of Alpha feed
 LLM   ── research               assistant pane fed by a real context handoff
```

Every screen is a FIXED grid repainted in place (no scrollback): header,
mode line, body, then status + a context-sensitive hint line. The **mode
line** always shows: screen tag, active venue, `RO`/`PO` modifier toggles
(loud red when on), the armed size preset, and the active watchlist; the
pair screen adds the ARMED symbol, the switcher/venue-picker add their
capture prompt. Keys live in ONE table (`ui/keymap.go`): the dispatcher,
the hint line, and the `?` overlay are all generated from it; verbs are
rebindable via `RSX_TERM_KEYMAP` (JSON `{"action":"key"}`).

### BOOK — the depth heatmap

```
 RSX  PENGU-PERP   ● live  ◀ bids  mid 0.010000  asks ▶
 BOOK  rsx  RO off  PO off  size 1.0000 [1]  list rsx
│                                                       −1h ┆ ■ 0.010001
│                                                      −10m ┆ ● 0.010001
│                                                       −1m ┆ ● 0.010001
│                                     ▒░░▒             −10s ┆
│                                     ▒░░▒                  ┆
│                                     ▒░●▓                  ┆   ← trade ● at its price
│                                     ▒░●█                  ┆   ← ask wall thickening
│                                     ▒░■█                  ┆
│                                     ◇▁▁█              now ┆   ← NOW row: micro-bars + your ◇
│─────────────────────────────────────▲┃┼──────────────
 ask 0.010001×6.0000  0.010002×40.0000
 bid 0.009999×5.0000  0.009998×8.0000
 pos LONG +3.0000 @ 0.009998   ~uPnL +0.000006
 ⚡ RTT 10.4 µs   p50 9.9 µs · p99 10.4 µs · best 9.0 µs
 x symbol  n news  b/s side  h/l cursor  j/k touch  f place  d cancel  q quit  tab view  r RO  ? help
```

- **Time axis (vertical, multi-resolution).** Bottom = the NOW row, the
  current book repainted every frame — it never scrolls. Above it, live
  ~100ms rows; above those, each far row aggregates an exponentially longer
  window on a fixed schedule (10s, 60s, 120s, 300s, 600s, then hours —
  `book.FarSpan`), labelled in the right gutter. Near = exact levels; far =
  time-weighted liquidity profile of the whole book. History survives a
  resize (rows are price-space).
- **Price axis (horizontal, fisheye).** Mid-centred, bids left / asks right,
  1 tick/cell at the touch, deep levels aggregating into edge columns —
  every row reads near-touch detail AND whole-book depth. Recenters on
  drift only (hysteresis), and re-aligns history too (render-time mapping).
- **Cell channels** — see VISUALS.md for the full glyph legend: background
  = resting size on ONE sequential ramp (side is position, never colour);
  glyph density ░▒▓█ = order count; ▚ = long-standing liquidity (L2
  persistence proxy, ≥30s); trade prints overlay in the AGGRESSOR's hue
  with a magnitude ramp ○◆●■; your orders are ◇ on the map and ▲/▼ on the
  ruler; the price cursor is ┃.
- **Trade tape rail** (right): the same prints as a feed, newest at top —
  magnitude glyph + exact price, aggressor-hued.
- **News rail** (left): severity-graded marker per row window (`· ► ► ‼`,
  muted→red); the headline itself lives in the NEWS screen.
- **Game order entry:** `b`/`s` side · `1-5` size presets · `h/l` cursor a
  tick, `j/k` to the touch · `f` places a resting limit at
  the cursor · `⇧1-5` crosses NOW (IOC at the far touch) · `d` cancels the
  own order nearest the cursor. ONE keypress fires — the qty cap and the
  notional ceiling still hard-block (`BLOCKED: …`).
- **`x` symbol switcher:** type the symbol's letter code (mode line shows
  candidates); hopping is instant — every watched market folds its own
  heatmap in the background. **`F9` venue picker:** switch the book between
  venues (rsx / hyperliquid); read-only venues render fully but BLOCK
  orders with the reason.
- **`n`** jumps to NEWS.

States: sizing (before the first WindowSizeMsg), `● offline` link dot,
empty side reads `ask —` in the touch ladder, `pos flat — fills build it`.

### PAIR — breadth, aggressive lots

```
 PAIR  rsx  RO off  PO off  size 1.0000 [1]  list rsx  ARMED WIF-PERP ×3
 [e] PENGU-PERP      0.010000    +0.32%  ▄▄  ██    L+2.0
 [w] WIF-PERP        0.021002    -1.10%  ▁█  ██    S-1.0
  │        │             │          │     │   │      └ position, in LOTS
  │        │             │          │     │   └ flow bar: intensity, hue = dominant aggressor
  │        │             │          │     └ depth state: bid|ask thickness vs its own past
  │        │             │          └ move vs session reference
  └ [letter] — press to ARM
```

Grammar: a **letter arms** its row (or `j`/`k` steps the arm); digits set a
vim-style **count**; `b` buys (lifts the offer), `s` sells (hits the bid) —
count × lots, always IOC; `.` flattens reduce-only; `[`/`]` cycle
watchlists (each list is one venue's universe — with Hyperliquid configured
the breadth venue's list is first); `esc` clears. **1 lot = RSX_TERM_LOT
notional × the symbol's multiplier** (`RSX_TERM_WATCH id:name:code:multPct`)
— a consistent risk unit across symbols. No passive quoting here; that is
the BOOK's job.

### NEWS — sector map + feed

```
 NEWS  rsx  RO off  PO off  size 1.0000 [1]  list rsx
 majors   SOL    +0.45%   BTC    -0.12%
 meme     PENGU  +2.10%
 ── news feed ── / search · j/k select · enter → assistant ──
▸ 03:33:20 ‼ Exchange halts withdrawals
  01:46:40 ► Binance lists SOL pair [SOLUSDT]
```

Tiles = the breadth venue's symbols grouped by sector, coloured by move vs
the session reference on a diverging bid/ask-hue scale (colorblind theme
swaps it to blue/orange). Feed = Tree of Alpha (`RSX_TERM_NEWS=1`; off =
labelled off, never an error), severity-graded, `/`-searchable (search
captures every key — `q` types, not quits). `enter` hands the selected
headline to the assistant; a symbol's **letter** dives straight into its
BOOK.

### LLM — the assistant pane

```
  ASSISTANT  (no model wired — the context below is exactly what one will receive)
  ── context handed off ─────────────────────────
  market   rsx · SOL-PERP  at 05:49:33
  headline ► Binance lists SOL pair
  book     mid 150.0000  (1 bid / 1 ask levels frozen)
  asks 150.0050×35.000000
  bids 149.9950×40.000000
  ── assistant reply ────────────────────────────
  ~ placeholder — wiring an LLM is a follow-up; nothing here is generated
```

The model is a PLACEHOLDER; the handoff is REAL: `news.AssistantContext`
{venue, symbol, timestamp, headline, deep-copied book snapshot, mid} — the
exact contract a wired model receives. `esc` returns to NEWS.

### Venues

`RSX_TERM_VENUE=rsx` (default, no external dials) · `hyperliquid`
(standalone read-only terminal over Hyperliquid market data — all four
screens work; orders are honestly BLOCKED until HL signing lands) · `both`
(RSX primary + HL breadth; PAIR/NEWS default to the HL universe, `F9`
switches the BOOK's venue). HL coins: `RSX_TERM_HL_COINS` (default curated
24, `all` for the whole universe).
