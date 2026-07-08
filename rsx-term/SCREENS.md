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
 RSX  PENGU-PERP   ● live   open 2  fills 7   last 10001  ~mark 10000 (mid)  index —  funding —
┌────────── book ──────────┐┌───────── order ─────────┐┌──── positions (mark=mid) ────┐
│ 10004  12 ▊▊▊▊           ││   BUY      SELL          ││ sym  side  net  entry  ~uPnL  │
│ 10003   8 ▊▊▊            ││                          ││ PENGU-PERP LONG +14 9998 +28 │
│ 10002  20 ▊▊▊▊▊▊▊        ││   price: 10001_          │└──────────────────────────────┘
│ 10001   5 ▊▊             ││   qty  : 5               │┌──────── trades ──────────────┐
│ — 1 —                    ││   time-in-force: GTC     ││ B 10001 5                    │
│ 10000   7 ▊▊▊            ││   reduce-only: off  …    ││ S 10000 3                    │
│  9999  15 ▊▊▊▊▊          ││                          ││ B  9999 8                    │
│  9998   9 ▊▊▊            ││   enter → preview, …     ││                              │
└──────────────────────────┘└──────────────────────────┘└──────────────────────────────┘
 ⚡ RTT 10.4 µs = net 2.5 µs + internal 7.6 µs + engine 0.34 µs    p50 9.9 µs · best 9.6 µs
 sent Buy 5 @ 10001 [GTC]
 q quit  b/s side  t tif  r ro  p po  tab field  0-9 type  ⌫ del  enter submit  c cancel  x flatten  F3 trace
```

- **status bar** — violet symbol badge; green `● live` link dot; open/fills
  counts; `last` (last trade), `~mark N (mid)` in dim italic (a client
  estimate, not exchange mark), `index —`, `funding —` (no source yet).
- **book** — asks worst-first down to the `— spread —` divider, then bids
  best-first; `▊` depth bar per lot (cap 24). Asks red, bids green.
- **order** — side toggle (active side reversed), price/qty fields (focused
  one bold+bright with `_`), time-in-force, reduce-only/post-only.
- **positions** — `mark=mid` in the title flags the derived mark; `LONG`
  green / `SHORT` red word; net, entry, `~uPnL` (dim-italic header, coloured
  value).
- **trades** — newest first, `B`/`S` glyph + price/qty, coloured by side.
- **speed strip** — the ⚡ round-trip split net / internal / engine + rolling
  p50 / best.

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
