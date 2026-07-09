# 44 — rsx-term: make it a superior trading terminal

Synthesis of the web research (pro DOMs: TT MD Trader, Bookmap, Sierra,
NinjaTrader, Quantower; Bloomberg conventions; HFT latency dashboards;
fat-finger case law) + the pro-trader visual critique, into a wave plan.
Full research + critique in the `.ship/42-TERM-AUDITS/`-style task outputs.

## Verdict going in

Already strong (keep, don't churn): two-`enter` confirm preview, reduce-only
flatten, strict colour=meaning palette, `~`/dim derived-value honesty, per-hop
latency strip + p50/p99/best + sparkline. The gaps below are what separate a
correct, honest depth *viewer* from a *pro ladder*.

## The single most impactful sequence (research verbatim)

static ladder → your orders/position/last-trade drawn *in* it →
right-aligned columns → responsive reflow → visual latency hop-bar.

## Waves

- **Wave 1 — DONE (`45a0b6c`)**: right-aligned fixed-width numeric columns;
  responsive reflow (ladder/tape depth from `WindowSizeMsg`); spread-row
  emphasis. (research must-haves #4, #5)
- **Wave 2 — in flight**: working-orders panel; cancel a *specific* order
  (not blind cancel-newest); own-order + last-trade markers; establishes the
  model tracking (#2, #3) markers re-home into the static ladder in wave 3.
- **Wave 3 — static price ladder (THE defining change, #1)**: rebuild
  `viewBook` around a **fixed price axis** — bid-qty-left / price-centre /
  ask-qty-right, quantities slide along a *stationary* price column, no
  per-tick reshuffle, recenter on a hotkey / when price nears the edge
  (TT/Sierra pattern). Integrate wave-2's own-order/position-entry/last-trade
  markers *into* the ladder rows. Add a top-of-book imbalance bar (#11).
  Opus — it's a structural rewrite of the book view.
- **Wave 4 — decimal/tick formatting (#1 "amateur" tell)**: query the
  webproto `Metadata` frame (`SymbolMeta{tick, lot, name}`, `49-webproto.md`)
  on connect; format raw i64 px/qty as human decimals at the display
  boundary; env/default fallback for the mock. Prices read `100.01`, not
  `10001`.
- **Wave 5 — latency as a visual (the differentiator, #7)**: a live stacked
  hop-bar (net│internal│engine widths proportional) + p99.9 + SLA-breach
  colouring (p99 over threshold → amber, 2× → red); keep the one-line strip
  calm, push the histogram/p99.9/sample-count into F3 (progressive
  disclosure). Label where each leg is measured.
- **Wave 6 — keyboard ergonomics + safety**: action hotkeys (`X` cancel-all,
  `R` reverse, `j`/`k` join bid/ask, `+`/`-` improve a tick, `m` market IOC);
  keyboard ladder cursor (`↑/↓` + `b/s` = click-to-price analog); qty
  steppers/presets; **fat-finger hard guard** (max-notional/size threshold
  that *forces* confirm / blocks — the Citi lesson, hard-block not
  soft-warn); `?` help overlay with destructive keys flagged; confirm on/off
  mode with a persistent **ARMED** banner when off.
- **Server-gated scaffold** (numbers dashed until backend lands): liq price +
  margin-health bar + ROE% + funding countdown given visual weight on the
  position (`59-latency-observability.md` for the internal/engine legs;
  risk/mark/funding specs for the rest).

## Skip (don't translate to a TTY)

2D time×price heatmap, hover tooltips, candlestick charts, multi-window
floating DOMs, Nerd-Font glyphs (use core Unicode blocks only).

## Discipline held across all waves

Colour = meaning (no new decorative colour); never fabricate a number (dash /
`·· pending`); stable keybindings; keep the mock demo working; every wave
`go build`/`vet`/`test`/`gofmt` green before commit.

## Shipped (2026-07-09)

- **Wave 1** `45a0b6c` — fixed-width aligned columns, adaptive resize, spread emphasis.
- **Wave 2** `98adce5` + `9047f4d` — own-order `▸` / last-trade `‹` markers, working-orders panel, cancel-by-selection (`↑↓`), `X` cancel-all.
- **Wave 3** `d125396` + `95899fb` — **static price ladder** (fixed axis, bid-left/ask-right, recenter-on-drift, gaps show liquidity) + top-of-book imbalance gauge. The defining pro-DOM change.
- **Wave 4** `03f521a` — **decimal/tick formatting**: raw i64 → human decimals at the display boundary (`fmtDec`/`strWidth`), PENGU 6/4 via env (`RSX_TUI_PRICE_DECIMALS`/`QTY_DECIMALS`). Ladder/tape/orders/positions/confirm/status/trace all read `0.010001`, not `10001`. The last "amateur tell" closed. Wire stays raw i64 (CLAUDE.md: convert only at the boundary).
- **Wave 5** `ca39816` — latency stacked hop-bar (`net│internal│engine`) + SLA-breach RTT colour.
- **Wave 6** `967ec00` `e6275c3` `2ec951f` — `m` market, `?` help overlay, `R` reverse, fat-finger hard-guard.
- **Docs** `203d47f` — SCREENS/VISUALS synced.

All `go build`/`vet`/`test`/`-race` green.

## Shipped (2026-07-09, part 2)

The whole terminal-side plan is now landed:

- **Wave 4** `03f521a` — decimal/tick formatting: raw i64 → human decimals at the
  render boundary (`fmtDec`/`strWidth`), PENGU 6/4 via env; wire stays raw.
- **Decimal input** `370b8d8` — the coherent other half: `parseRaw` (fmtDec's
  inverse) + `.` key, so you *type the price you read* (`0.010001`, not `10001`).
  Fixed the read-decimal/type-raw mismatch (qty "5" previewed `0.0005`).
- **Wave 6 tail** `961c59f` — `+/-` improve-tick (seeds from mid), `j/k` join
  best bid/ask, `F2` ARMED confirm-off (single-enter fire, loud red banner; the
  fat-finger guard still holds).
- **Depth bars** `3b9b7dd` — restored the spec's `▊` per-level histogram (bid
  grows left, ask right, converging on the spread).
- **Positions** `b21ee10` — quote-precision uPnL + wrap-safe stacked layout +
  honestly-dashed liq/ROE/margin-health risk row.
- **Mouse click-to-price** `69e77db` — left-click a ladder row sets its price
  (`priceAtY`, shared `resolvedCenter`); never submits.
- **Responsive** `59ebf2d` — narrow terminals stack panels vertically (wired the
  dead `narrow()`).
- **Colorblind theme** `c864d97` — `RSX_TUI_THEME=colorblind` swaps bid/ask to a
  deuteranopia-safe blue/orange.
- **Confirm fix** `23be93e` — the preview no longer overflows the order panel.
- Spec 55 + SCREENS/VISUALS reconciled to the actual UI at each step.

## Still backend-gated (NOT terminal-side; honestly dashed today)

- liq price, margin-health, ROE%, funding countdown, and the live
  `internal`/`engine` latency legs — all need risk/mark/funding server work
  (`59-latency-observability.md` inc 3). The terminal already gives them a fixed,
  dashed home; wiring is a server task, not a terminal one.

## Consciously skipped (low value for this instrument)

- cumulative-depth toggle (per-level depth bars already show the shape),
  tick grouping (PENGU is tick-1). Revisit only if a coarser-tick symbol lands.
