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

## Remaining (queued, not terminal-side-blocked-solo)

- **Wave 4 — decimal/tick formatting**: the raw-`i64` display. Needs the webproto `Metadata` frame (real per-symbol tick/lot) — hardcoding risks a *wrong* display, so this is queued for opus with a live cluster to verify (mock falls back to raw honestly). The last terminal-side "amateur tell".
- **Wave 6 tail**: `+/-` improve-tick (needs the tick from wave 4), qty presets, confirm-off ARMED mode.
- **Server-gated scaffolds** (honestly dashed today, no backend): liq price, margin-health bar, ROE%, funding countdown, and the live `internal`/`engine` latency legs (spec 59 inc 3).
- **Polish**: cumulative-depth toggle, tick grouping, mouse click-to-price, colorblind theme.
