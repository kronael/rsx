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
