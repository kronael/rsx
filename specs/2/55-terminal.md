# 55 — Trade Terminal (rsx-term) UX

Status: **partial**. The single-symbol perps terminal ships in code
(`rsx-term`, Go / Bubble Tea): live order form, ladder, trade tape,
derived position, and the speed strip render against both the offline mock
and a live cluster. This spec is the **UX contract**: the perps screen in
full, a new-trader safety bar, and the multi-market
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
- [New-trader safety requirements](#new-trader-safety-requirements)
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
  submit confirmation outrank keystroke count (see the safety requirements).

## Transport & data sources

The terminal (`rsx-term`, Go / Bubble Tea) connects over **WebSocket**
(`coder/websocket`) — two independent sockets, mirroring the reference
client `rsx-playground/market_maker.py`:

- **Private (authenticated):** order submit/cancel and the account's own
  updates — `N`/`C` out, `U`/`F`/`E`/`H` in, as **JSON text frames**
  (`49-webproto.md` §Order Messages). Auth is a `Bearer <JWT>` header
  (`54` identity model).
- **Public (unauthenticated):** market data — subscribe `{S:[sym,7]}`
  (bbo|depth|trades); receive `B`/`D`/`BBO`/`T` as **protobuf binary
  frames** (`49` §Market Data, `rsx-marketdata/marketdata.proto`). The
  ladder and tape come from here.

The two links reconnect independently with backoff; a marketdata drop
never takes the order link down.

Account queries `O`/`P`/`A`/`FL`/`FN` (`49`) are **Post-MVP: not
implemented in v1**. Until they land, positions/uPnL are **derived
client-side** from the account's own fills plus a book-mid mark; account
equity/margin, funding, and liquidation price have **no source** and show
`—`. These are the gaps the honesty table tracks.

## Palette

The terminal uses the dashboard's **Ayam Cemani** palette — defined once
in `rsx-term/ui/styles.go` as Lipgloss colours, mirroring the Tailwind
retune in `rsx-playground/pages.py` and the semantics in
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

## Perps terminal

Three columns under a status bar, over a speed strip + status line + help.
This is the perps screen `rsx-term/ui/view.go` renders. Fields whose data
has no server source yet are marked **[needs server]**; what's sourced vs
not is enumerated in the data-source table below.

Prices/qtys are shown as human decimals (raw i64 / 10^decimals, e.g. PENGU
`10001` → `0.010001`); the wire stays raw i64. The ladder is a **static price
axis** — bid qty left, ask qty right of a fixed price column that recentres
only on drift, so it doesn't reshuffle every tick (TT/Sierra pattern).

```
 RSX  PENGU-PERP   ● live  open 1  fills 0   last 0.010000  ~mark 0.010002 (mid)  index —  funding —
┌──────────── book ────────────┐┌────────── order ──────────┐┌───── positions (mark=mid) ──────┐
│              0.010005 12 ▊▊   ││  BUY      SELL             ││ LONG  +15.0000 @ 0.009999       │
│              0.010004  8 ▊    ││                            ││ ~uPnL +0.000045                 │
│              0.010003 25 ▊▊▊▊▊││ price: 10001_              ││ liq —  ROE —  mgn ░░░░░░░░       │
│              0.010002  6 ▊    ││ qty  : 5                   ││        (needs risk engine)      │
│              0.010001         ││ time-in-force: GTC         │└─────────────────────────────────┘
│‹      7 0.010000              ││ reduce-only: off  post:off │┌──────── orders ─────────────────┐
│     ▊▊▊ 15 0.009999           ││                            ││ ▸ BUY  0.009998 9.0000          │
│▸      9 0.009998              ││ enter → preview → enter    │└─────────────────────────────────┘
│  ▊▊▊▊▊▊ 30 0.009997           ││                            │┌──────── trades ─────────────────┐
│████████████ 54% bid           ││                            ││ S 0.010000 2.5000               │
└───────────────────────────────┘└────────────────────────────┘└─────────────────────────────────┘
 ⚡ RTT 10.44 µs = net 2.5 µs + internal 7.6 µs + engine 0.34 µs      p50 9.9 µs · best 9.6 µs
 sent BUY 5 @ 10001 [GTC]
 q quit  b/s side  t tif  r ro  p po  +/- tick  j/k join  tab field  enter submit  m mkt  c cancel  X all  x flatten  R reverse  F2 armed  F3 trace  ? help
```

### Panels

- **book** — a static price axis: **bid qty left, ask qty right** of a fixed
  price column. Each level draws a depth bar (`▊`) scaled to the deepest
  visible level, in the side colour; bid depth grows left and ask right, so
  the two bars converge on the spread (DOM/Bookmap depth read). The empty
  band between best bid and best ask *is* the spread — gaps show where
  liquidity isn't. Own orders `▸` and the last print `‹` mark their rows; a
  top-of-book **imbalance bar** closes the panel. When `connected` but the
  ladder is empty, show an amber `no live book — market-data stream down`
  row (degraded, not blank).
 
- **order** — the entry form. `b`/`s` pick side (reversed-highlight the
  active one); `price`/`qty` accept **human decimals** (`0.010001`, not raw
  `10001`) reconstructed to raw i64 at submit (`tab` switches focus, `.`
  adds the point); `t` cycles TIF (GTC→IOC→FOK); `r`/`p` toggle
  **reduce-only** / **post-only** (the `ro`/`po` fields exist on the `N`
  frame, `49`).
  `+`/`-` nudge the price one tick (seeded from mid when empty); `j`/`k`
  join the best bid/ask; `m` sends a **market** IOC at the far touch.
  Every order path routes through a two-enter **confirm preview** and a
  **fat-finger hard guard** (size over the cap is blocked outright, not
  soft-warned). `F2` toggles **ARMED** (confirm-off, single-enter fire)
  behind a persistent red banner — the size guard still holds. Leverage /
  margin-mode selectors are display-only until risk exposes them.
  **[needs server]**
- **positions** — the account's open position(s), stacked so the narrow
  column never wraps: side + signed net qty `@` entry, then `~uPnL`
  (green/red, money at quote precision), then a **risk row** — liq price,
  ROE%, and a margin-health bar — honestly dashed (`StyleDerived`) until
  the risk feed lands, so the whole risk surface has a fixed home without
  fabricating a number. **[position built client-derived; risk row needs
  server]** Title is `positions (mark=mid)` to flag the mark as
  book-mid-derived.
- **trades** — the public tape, newest first, price coloured by taker
  side.
- **status bar** — symbol badge, link dot (`● live`/`● offline`), open /
  fills counters. Extend with `last` (from the tape), `mark`
  (mid-derived, labelled), `index`, and a **funding** rate + countdown.
  **[needs server]**
- **speed strip** — the ⚡ round-trip, split net / internal / engine, with
  rolling p50 / best. The terminal's signature: it *shows* the
  µs-class path other exchanges hide.
- **status line / help** — last event; keybinding legend.

### Order lifecycle & confirmation

`enter` currently submits immediately. The target flow adds a
**confirmation preview** (the single biggest new-trader guardrail): the
first `enter` renders a preview line — side, qty, notional (`px*qty`),
TIF, reduce-only, and the **resulting liquidation price** — and a second
`enter` sends, `esc` cancels. **[near-term for the preview; liq needs
server]** A `c` key cancels the selected/last resting order via the `C`
frame (`49`).

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

## New-trader safety requirements

A first-time trader must not lose money to a confusing screen. The terminal
makes these unmissable, ranked by how badly a beginner is hurt without each:

1. **Liquidation price** on the open position — the "you're wiped out here"
   number, never buried.
2. **Unmistakable side** — colour plus a LONG/SHORT word, everywhere.
3. **uPnL in $ and ROE%**, live and colour-coded.
4. **Available margin / free balance** — the over-sizing guard.
5. **Confirm-before-submit** preview — side, size, notional, resulting liq
   — the single biggest fat-finger guard.
6. **Leverage** shown next to size.
7. **Mark vs last** both labelled; a client-derived value (a mid-derived
   mark, or uPnL off it) is rendered as a visible *estimate*, not with the
   confidence of exchange data.

Should-have: funding rate + countdown, margin-ratio / distance-to-liq bar,
size as % of balance, reduce-only as the default on a close. Fields with no
server source (liq, margin, leverage, ROE%, funding) are dashed, not faked.

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
