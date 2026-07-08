# 55 — Trade Terminal (rsx-tui) UX

Status: **draft**. The single-symbol perps terminal exists in code
(`rsx-tui`, ratatui); the order form, ladder, positions table, and speed
strip render today. This spec is the **target UX**: the perps screen in
full, a new-trader (first-time) requirements bar, and the multi-market
vision (account · perps · options · structured derivatives · lending)
with screen mockups. Fields that have no data source yet are labelled as
gaps, not dressed up.

Companion specs: `49-webproto.md` (the wire the terminal speaks),
`54-tui-access.md` (how a trader gets a session — SSH / web), `28-risk.md`
(margin, liquidation), `16-marketdata.md` (book/BBO/trades fan-out),
`15-mark.md` (mark price).

## Table of Contents

- [Principles](#principles)
- [Transport & data sources](#transport--data-sources)
- [Palette](#palette)
- [Perps terminal (the built screen)](#perps-terminal-the-built-screen)
- [New-trader requirements & current scorecard](#new-trader-requirements--current-scorecard)
- [Multi-market vision](#multi-market-vision)
  - [Account Management](#account-management)
  - [Options](#options)
  - [Structured Derivatives (sfdx)](#structured-derivatives-sfdx)
  - [Lending Markets](#lending-markets)
- [Data-source honesty table](#data-source-honesty-table)

---

## Principles

- **One screen, one instrument (today).** The terminal trades a single
  symbol (`PENGU-PERP`). Multi-market is a navigation layer added later
  (a market switcher), not a rewrite — every screen below reuses the same
  ladder / form / positions primitives.
- **Colour is meaning, never decoration.** Green = live/long/filled, red
  = short/down/reject, amber = degraded/stale, violet = heading/accent.
  Shared verbatim with the dashboard (see [Palette](#palette)).
- **Never fabricate a number.** A field with no data source shows `—`,
  never a plausible-looking `0`. This already holds for the latency strip
  (`net_ns: Option`); it extends to every derived/absent field here.
- **Honest labels for derived values.** Where the terminal computes a
  value the server does not send (e.g. a mid-derived mark, a
  client-tracked position), the label says so (`mark=mid`), so nobody
  reads a client estimate as an exchange figure.
- **Guardrails before ergonomics.** A first-time trader must not lose
  money to a confusing screen: liquidation price, side, and a
  submit confirmation outrank keystroke count (see the scorecard).

## Transport & data sources

Per `54-tui-access.md`, the terminal speaks **protobuf-over-QUIC** to the
gateway (the WebSocket client is being retired). Two logical streams,
regardless of framing:

- **Private (authenticated):** order submit/cancel and the account's own
  updates — `N`/`C` out; `U`/`F`/`E`/`H` in (`49-webproto.md` §Order
  Messages). Carries the trader's identity (`54` identity model).
- **Public (unauthenticated):** market data — subscribe `{S:[sym,7]}`
  (bbo|depth|trades); receive `B`/`D`/`BBO`/`T` (`49` §Market Data). The
  ladder and tape come from here.

Account queries `O`/`P`/`A`/`FL`/`FN` (`49`) are **Post-MVP: not
implemented in v1**. Until they land, positions/uPnL are **derived
client-side** from the account's own fills plus a book-mid mark; account
equity/margin, funding, and liquidation price have **no source** and show
`—`. These are the gaps the honesty table tracks.

## Palette

The terminal uses the dashboard's **Ayam Cemani** palette — defined once
in `rsx-tui/src/palette.rs` as `ratatui` RGB consts, mirroring the
Tailwind retune in `rsx-playground/pages.py` and the semantics in
`rsx-playground/CLAUDE.md`. Do **not** invent terminal-only colours; add
one only when it maps to a new *meaning*.

| Meaning | Const | Hex |
|---|---|---|
| live / long / bid / filled | `LIVE` = `BID` | `#22f5a1` |
| short / ask / down / reject | `ASK` | `#f87171` |
| heading / badge / ⚡ accent | `HEADING` | `#bd83ff` |
| info / secondary accent | `ACCENT` | `#a992ff` |
| overlay ring | `RING` | `#7c3aed` |
| body text | `TEXT` | `#a9bcb2` |
| bright / focused / statusline | `TEXT_BRIGHT` | `#e7eeea` |
| muted / labels / help / dim | `MUTED` | `#586b62` |
| degraded / stale / offline | `DEGRADED` | `#fbbf24` |
| panel bg / page bg / border | `PANEL_BG` `#0d1712` · `PAGE_BG` `#040806` · `BORDER` `#16211b` |

## Perps terminal (the built screen)

Three columns under a status bar, over a speed strip + status line + help.
This is what `rsx-tui/src/render.rs` draws; new elements below are marked
**[built]** / **[near-term]** / **[needs server]**.

```
 RSX  PENGU-PERP    ● live      open 2   fills 7          last 10001  mark 10000  index —   funding — in —:—:—
┌──────── book ─────────┐┌──────── order ────────┐┌────── positions (mark=mid) ─────┐
│ 10004    12  ▊▊▊▊     ││                        ││ sym         net  entry   uPnL   │
│ 10003     8  ▊▊▊      ││   BUY      SELL         ││ PENGU-PERP  +14   9998   +28    │
│ 10002    20  ▊▊▊▊▊▊▊  ││                        ││                                 │
│ 10001     5  ▊▊       ││   price:  10001_        ││ (ROE% · liq — [needs server])   │
│ — 1 —                 ││   qty  :  5             │└─────────────────────────────────┘
│ 10000     7  ▊▊▊      ││   tif  :  GTC           │┌──────── trades ─────────────────┐
│  9999    15  ▊▊▊▊▊    ││   ro   :  off  po: off  ││  10001   5                       │
│  9998     9  ▊▊▊      ││                         ││  10000   3                       │
│  9997    30  ▊▊▊▊▊▊▊  ││   enter → confirm       ││   9999   8                       │
└───────────────────────┘└─────────────────────────┘└─────────────────────────────────┘
 ⚡ RTT 10.44 µs = net 2.5 µs + internal 7.6 µs + engine 0.34 µs      p50 9.9 µs · best 9.6 µs
 sent Buy 5 @ 10001 [GTC]
 q quit  b/s side  t tif  r ro  p po  tab field  0-9 type  ⌫ del  enter submit  c cancel  F3 trace
```

### Panels

- **book** — asks (red) descending to a spread row, bids (green)
  ascending; a unicode depth bar (`▊`) scaled to resting qty. **[built]**
  When `connected` but the ladder is empty, show an amber
  `no live book — market-data stream down` row (degraded, not blank).
  **[near-term]**
- **order** — the entry form. `b`/`s` pick side (reversed-highlight the
  active one); `price`/`qty` are digit buffers (`tab` switches focus);
  `t` cycles TIF (GTC→IOC→FOK). **[built]** Add `r`/`p` to toggle
  **reduce-only** / **post-only** (the `ro`/`po` fields already exist on
  the `N` frame, `49`). **[near-term]** A **market** convenience (send IOC
  at the far touch) is a TIF-adjacent option. **[near-term]** Leverage /
  margin-mode selectors are display-only until risk exposes them.
  **[needs server]**
- **positions** — the account's open position(s): symbol, signed net
  qty, avg entry, uPnL (green/red). **[built, client-derived]** Title is
  `positions (mark=mid)` to flag the mark as book-mid-derived. Add ROE%
  (`uPnL / margin`) and **liquidation price** once margin data exists.
  **[needs server]**
- **trades** — the public tape, newest first, price coloured by taker
  side. **[built]**
- **status bar** — symbol badge, link dot (`● live`/`● offline`), open /
  fills counters. **[built]** Extend with `last` (from the tape), `mark`
  (mid-derived, labelled), `index`, and a **funding** rate + countdown.
  **[needs server]**
- **speed strip** — the ⚡ round-trip, split net / internal / engine, with
  rolling p50 / best. **[built]** The terminal's signature: it *shows* the
  µs-class path other exchanges hide.
- **status line / help** — last event; keybinding legend. **[built]**

### Order lifecycle & confirmation

`enter` currently submits immediately. **[built]** The target flow adds a
**confirmation preview** (the single biggest new-trader guardrail): the
first `enter` renders a preview line — side, qty, notional (`px*qty`),
TIF, reduce-only, and the **resulting liquidation price** — and a second
`enter` sends, `esc` cancels. **[near-term for the preview; liq needs
server]** A `c` key cancels the selected/last resting order via the `C`
frame (`49`). **[near-term]**

### Keybindings

| key | action | state |
|---|---|---|
| `q` / `esc` | quit (esc also cancels a pending confirm) | built |
| `b` / `s` | side buy / sell | built |
| `t` | cycle TIF GTC→IOC→FOK | built |
| `tab` | switch price/qty focus | built |
| `0`-`9` / `⌫` | edit focused field | built |
| `enter` | submit (→ confirm → send) | built (confirm near-term) |
| `r` / `p` | toggle reduce-only / post-only | near-term |
| `c` | cancel resting order | near-term |
| `F3` | trace HUD overlay | built |

## New-trader requirements & current scorecard

The bar a first-time trader needs to not lose money to confusion, ranked,
with where the terminal stands. (This is the "someone who has never traded
perps" review lens.)

**MUST-HAVE**

| # | requirement | status | note |
|---|---|---|---|
| 1 | **Liquidation price** always visible on the position | ✗ missing | needs risk margin data (`28-risk.md`); no source in `49` yet |
| 2 | **Unmistakable side** colour + label everywhere | ◑ partial | buy/sell coloured; long/short position sign shown, but no explicit "LONG/SHORT" word |
| 3 | **uPnL in $ and ROE%**, live, colour-coded | ◑ partial | $ uPnL derived & coloured; ROE% needs margin |
| 4 | **Available margin / free balance** | ✗ missing | needs `A` account query (Post-MVP) |
| 5 | **Confirm-before-submit** preview (side/size/notional/liq) | ✗ missing | near-term; liq field needs server |
| 6 | **Leverage shown** next to size | ✗ missing | leverage not in the order path |
| 7 | **Mark vs last** both labelled | ◑ partial | last from tape; mark shown as mid-derived, labelled; true mark needs `15-mark.md` feed |

**SHOULD-HAVE**

| # | requirement | status | note |
|---|---|---|---|
| 8 | Funding rate + countdown | ✗ missing | no funding field in `49` market data yet |
| 9 | Margin-ratio / distance-to-liq health bar | ✗ missing | needs margin |
| 10 | Size as % of balance | ✗ missing | needs balance |
| 11 | Reduce-only default on close actions | ✗ missing | pairs with the `r` toggle + a close action |
| 12 | Spread / BBO visible | ✓ done | spread row + ladder |

**NICE-TO-HAVE:** trades tape ✓ · open-interest / 24h stats ✗ · price
sparkline ✗ · maker/taker fee estimate on preview ✗.

The gating theme: the terminal's *market-data and execution* half is
solid; its *risk/account* half (liq, margin, leverage, funding) is blocked
on server features that are Post-MVP. Those are the highest-value next
builds for trader safety — call them out rather than fake them.

## Multi-market vision

One navigation layer (a market switcher: `[ perps | options | sfdx |
lend | acct ]`) over screens that reuse the ladder/form/table primitives.
Each screen below is a target mock; all instrument/account data beyond the
perps book is **[needs server]** today.

### Account Management

The portfolio-margin home: balances, cross-margin health, every open
position across markets, and working orders. Portfolio margin (one
collateral pool backing all positions) is the RSX model (`28-risk.md`).

```
 RSX  ACCOUNT  user 1                                          ● live
┌──────── balances ─────────────┐┌──────── margin health ─────────────┐
│ collateral      12,500.00     ││ equity          12,742.00          │
│ equity          12,742.00     ││ init margin      3,050.00 (24%)    │
│ unrealized      +242.00       ││ maint margin     1,220.00          │
│ available        9,450.00     ││ margin ratio  [███████░░░] 61%     │
└───────────────────────────────┘└─────────  100% = liquidation ──────┘
┌──────── positions (all markets) ───────────────────────────────────┐
│ market        side   size   entry    mark    liq     uPnL    ROE%   │
│ PENGU-PERP    LONG    +14    9998   10000    9120    +28    +0.9%   │
│ BTC-25JUN-C   LONG    +2      340     372      —     +64    +18%    │
└─────────────────────────────────────────────────────────────────────┘
┌──────── working orders ────────────────────────────────────────────┐
│ id     market       side  px      qty  filled  tif   status         │
│ a3f1   PENGU-PERP   BUY   9997     30     0     GTC   resting        │
└─────────────────────────────────────────────────────────────────────┘
 x cancel   X cancel-all   enter → market   tab switch market   q quit
```

Fields: `collateral/equity/unrealized/available` ← `A` query; positions ←
`P` query; orders ← `O` query — all **[needs server]** (Post-MVP `49`).
Margin ratio = `maint_margin / equity`; the health bar goes amber >70%,
red >90%.

### Options

A strike ladder (chain) for one expiry, mark + greeks per strike, with the
same order form below. Options are the sfdx "2D (nonlinear payoff)" case,
but a conventional chain view is the familiar entry point.

```
 RSX  BTC-OPTIONS  exp 25-JUN   spot 67,420   iv 52%          ● live
┌──────── calls ────────┬─ strike ─┬──────── puts ─────────┐
│ mark  delta  bid  ask │          │ bid  ask  delta  mark │
│  920   .78  915  925  │  66000   │  40   44  -.22   42   │
│  540   .61  536  545  │  67000   │  62   66  -.39   64   │
│  310   .43  305  314  │  68000   │ 128  134  -.57  131   │   ← ATM band
│  150   .26  146  155  │  69000   │ 262  270  -.74  266   │
└───────────────────────┴──────────┴───────────────────────┘
 selected  BTC-25JUN-68000-C   mark 310  Δ.43 Γ.002 Θ-4.1 V.31
 [ order form — side/price/qty/tif, same primitives ]
 tab strike   c/p call/put   enter → confirm   q quit
```

Fields: per-strike bid/ask/mark from an options ME; greeks
(Δ/Γ/Θ/V) + IV from a pricing service. All **[needs server]** — no options
matching or pricing exists yet.

### Structured Derivatives (sfdx)

The krons flagship: a **basis-listing** exchange. The venue lists basis
functions `{φᵢ}`; a trader builds a custom payoff `f = Σ αᵢ φᵢ` by
choosing weights `α`. Liquidity pools across every payoff sharing a basis;
fees scale with `‖α‖₀` (the count of non-zero weights — sparsity), and
no-arbitrage is enforced across all derived functionals. Payoff
dimensions: 0D binary · 1D futures · 2D options · nD path-dependent · text
markets (LLM-scored outcomes from compressed world states).

```
 RSX  SfDx  basis: BTC-JUN-{φ}   fee ∝ ‖α‖₀ = 3            ● live
┌──────── basis functions φᵢ ───────────┐┌──── your payoff  f = Σ αᵢφᵢ ────┐
│  i  φ (basis)          implied  bid ask││  α₁ · φ(digital>70k)    +1.0     │
│  1  digital >70k        0.42   .41 .43 ││  α₂ · φ(linear 60–80k)  -0.5     │
│  2  linear 60–80k       0.50   .49 .51 ││  α₅ · φ(∫ path var)     +0.2     │
│  5  ∫ path variance     0.18   .17 .20 ││                                 │
│  9  text: "ETF approved" 0.61  .59 .63 ││  ‖α‖₀ = 3   fee 0.30%           │
└────────────────────────────────────────┘│  price(f) = Σαᵢ·mid  = 0.34     │
 quote payoff → arb implies basis quotes  │  settle: f(z) = Σαᵢφᵢ(z_actual) │
 +/- weight   n new leg   enter → quote   └──────────────────────────────────┘
```

Left: the listed basis with implied/bid/ask per `φᵢ`. Right: the payoff
builder — add legs (`n`), set weights (`+`/`-`), see `‖α‖₀`, the sparsity
fee, the composite price, and the settlement rule. All **[needs server]** —
this is the research frontier (the sfdx matching/pricing engine does not
exist in this repo yet; it is the intended product this exchange grows
into).

### Lending Markets

Collateral earns yield and backs borrowing; a rate curve per asset drives
the portfolio-margin collateral value. A conventional supply/borrow screen.

```
 RSX  LEND                                                    ● live
┌──────── markets ───────────────────────────────────────────────────┐
│ asset   supply APY  borrow APY  utilisation   your supply  your debt│
│ USDC       4.2%       6.8%     [██████░░] 74%    8,000.00      —     │
│ BTC        0.3%       1.1%     [██░░░░░░] 22%       —       0.15     │
└─────────────────────────────────────────────────────────────────────┘
 selected USDC   supply 8,000  earning 4.2%   health factor 2.4
 s supply   b borrow   w withdraw   r repay   enter → confirm   q quit
```

Fields: per-asset supply/borrow APY, utilisation, the account's
supply/debt, health factor. All **[needs server]** — no lending engine
exists; this reserves the UX slot in the portfolio-margin picture.

## Data-source honesty table

Every terminal field and whether it has a source today.

| field | source | status |
|---|---|---|
| book ladder, BBO, spread | public MD `B`/`D`/`BBO` (`49`) | **live** |
| trades tape, last price | public MD `T` (`49`) | **live** |
| net RTT (speed strip, net leg) | client-measured (submit→ack) | **live** |
| internal / engine latency legs | gateway-stamped | not stamped yet → `—` |
| order submit / accept / fill / done | private `N`/`U`/`F` (`49`) | **live** |
| order cancel | private `C` (`49`) | wire exists; key near-term |
| reduce-only / post-only | `N` frame `ro`/`po` (`49`) | wire exists; toggles near-term |
| position net / entry / uPnL ($) | **derived** from own fills + book-mid | **live (derived)** |
| mark price | book-mid (labelled) / `15-mark.md` feed | derived; true mark needs server |
| index price | risk size-weighted mid | **needs server** |
| ROE %, liquidation price, margin ratio | risk margin (`28-risk.md`) | **needs server** |
| account equity / available / collateral | `A` query (`49`) | **Post-MVP** |
| open orders / positions list (all markets) | `O` / `P` query (`49`) | **Post-MVP** |
| funding rate + countdown | funding engine + MD field | **needs server** |
| leverage / margin mode | risk account config | **needs server** |
| options chain / greeks / IV | options ME + pricing | **needs server** |
| sfdx basis / implied / payoff price | sfdx engine | **needs server (frontier)** |
| lending APY / utilisation / health | lending engine | **needs server** |

The pattern: **market data + execution are live; everything risk-,
account-, and new-market-shaped is a labelled gap.** The terminal never
invents those numbers — it shows `—` and this table says why.
