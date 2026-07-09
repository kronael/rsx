# SCREENS — rsx-term

Every screen and state the terminal renders, with a mockup and what it means.
One screen, one symbol (PENGU-PERP); states are variations of it, not separate
pages. Rendering lives in `rsx-term/ui/view.go`; styling in `VISUALS.md`.
Colours below are described by meaning (see the palette) — a terminal shows
them, this file can't.

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
bid/ask · `enter` preview → `enter` send (`esc` cancels) · `m` market (IOC
far touch) · `↑↓` select a working order · `c` cancel it · `X` cancel all ·
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

`b`/`s` pick side (active side reversed), `tab` moves focus, digits edit the
focused field (bold + bright + `_` cursor), `t` cycles TIF, `r`/`p` toggle
reduce-only / post-only:

```
┌───────── order ─────────┐
│   BUY      SELL          │   ← SELL reversed = active side
│                          │
│   price: 10001           │
│   qty  : 5_              │   ← focused field: bold, bright, cursor
│   time-in-force: IOC     │
│   reduce-only: on   post-only: off │
│                          │
│   enter → preview, enter again to send │
└──────────────────────────┘
```

## Confirm preview (submit guard)

First `enter` never submits — it renders a violet-ringed preview inside the
order panel. Second `enter` sends; `esc` cancels. `liq` has no server source
yet, shown as a deliberate `n/a` with one shared legend, not a fake number.

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
