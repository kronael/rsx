# FLOWS — rsx-term user walkthroughs

Every user journey through the terminal, step by step: the keys you press, the
events the exchange sends back, and what you see. Each flow is backed by a
scenario test in `ui/flows_test.go` (same number + name) that drives the model
through the whole journey — so this doc and the executable checks stay in
lockstep. Run them with `make test` (or `go test ./ui/ -run TestFlow`).

Prices/qtys below are PENGU-PERP (price 6 decimals, qty 4 decimals, tick 1):
you read and type `0.010001`; the wire carries the raw i64 `10001`.

The panels, keys, and colours these flows reference are catalogued in
`SCREENS.md` (states) and `VISUALS.md` (design system).

---

## 1. Startup & connect — `TestFlowStartupConnect`

1. Launch. Before marketdata arrives, the book panel shows an honest amber
   `no live book — market-data stream down` row (never a blank or fake ladder),
   and the link dot is `● offline`.
2. The order link comes up → status reads `connected`, dot goes `● live`.
3. The marketdata link comes up and a snapshot lands → the static price ladder
   renders with decimal prices and per-level depth bars.

## 2. Place a limit order — `TestFlowPlaceLimitOrder`

The core flow. `b`/`s` pick the side (Buy default).

1. Type the **price you read off the ladder**: `0.010001` (digits + `.`).
2. `tab` to the qty field, type `5`.
3. `enter` → a **confirm preview** appears inline in the order panel (side,
   size, price, notional, TIF). Nothing is sent yet.
4. `enter` again → the order is submitted as raw i64 (`Px 10001, Qty 50000`);
   the status line reads `sent BUY 5.0000 @ 0.010001 [GTC]`.
5. The gateway acks with an oid → the order joins the **working-orders** panel,
   and its resting level is marked `▸` on the ladder.

`esc` at step 3 cancels the preview without sending.

## 3. Cancel a specific working order — `TestFlowCancelSelected`

With two or more working orders:

1. `↑`/`↓` move the cursor (`▸`) through the working-orders panel.
2. `c` cancels the **selected** order — not a blind cancel-newest.
3. The exchange confirms with a done event → that one order leaves the panel;
   the others stay.

## 4. Cancel all — `TestFlowCancelAll`

`X` cancels every working order in one keystroke (a flagged, destructive key).

## 5. Market order — `TestFlowMarketOrder`

Take liquidity now, at the far touch:

1. `tab` to the qty field, type the size (market needs no price).
2. `m` → arms an **IOC at the far touch** (a Buy crosses the best ask) through
   the same confirm gate.
3. `enter` → submitted as IOC.

## 6. A fill builds a position — `TestFlowFillBuildsPosition`

When a fill event names one of your orders, it folds into the position:
`LONG`/`SHORT`, signed net, average entry, and `~uPnL` (mark = book mid,
flagged with `~`). The fills counter in the status bar ticks up. The risk row
(liq / ROE / margin-health) stays dashed until the risk engine feeds it.

## 7. Flatten — `TestFlowFlatten`

Close the whole position safely:

1. `x` → builds a **reduce-only** order that closes the net at the opposing
   touch (a long sells at the best bid), sized to `|net|`, through the confirm.
   Reduce-only means the exchange can only shrink the position toward flat — it
   can never overshoot past zero.
2. `enter` → submitted; the done event clears it.

A flat position, or no book to price against, is a no-op with a reason — never
a fabricated price.

## 8. Reverse — `TestFlowReverse`

`R` flips the position: a marketable order of **2× the net** on the opposite
side (a `+15` long → Sell `30`), deliberately crossing zero. Not reduce-only.
Through the confirm gate like everything else.

## 9. ARMED — confirm-off — `TestFlowArmedConfirmOff`

For fast trading, `F2` removes the two-enter step:

1. `F2` → a loud, persistent red **ARMED** banner appears; orders now fire on a
   single `enter`.
2. Type an order, `enter` once → submitted immediately (no preview).
3. `F2` again re-arms the safety.

ARMED removes the *confirm*, never the **fat-finger guard** — an oversized
order is still hard-blocked.

## 10. Mouse click-to-price — `TestFlowClickToPrice`

Left-click any ladder row → that row's price loads into the price field and it
takes focus. A click never submits (that would bypass the confirm). Disabled in
the narrow stacked layout, where the row offsets differ.

## 11. Price helpers — `TestFlowPriceHelpers`

- `j` / `k` — join the best bid / ask (loads that price).
- `+` / `-` — nudge the price one tick (seeded from the mid if the field is
  empty), floored at one tick.

## 12. Fat-finger block — `TestFlowFatFingerBlock`

An order over the size cap is refused outright — never previewed, never sent;
the status reads `BLOCKED: qty … exceeds max …`. A hard stop, not a dismissible
soft warning (the Citi lesson: soft warnings train click-through).

## 13. Reject — `TestFlowReject`

An exchange rejection surfaces in the status line with its reason
(`rejected: insufficient margin`).

## 14. Marketdata down & recovery — `TestFlowMarketdataDegraded`

If the marketdata link drops, the ladder degrades to the amber `no live book`
row rather than showing a stale or blank book. When the link recovers and a
fresh update lands, the ladder restores.

## 15. Gateway down — `TestFlowGatewayDegraded`

If the order link drops, the link dot flips to `● offline` and orders can't be
sent (auto-reconnect with backoff). The marketdata link is independent and
stays live — you keep seeing the book.

## 16. Overlays — `TestFlowOverlays`

- `F3` → the **latency trace** HUD (round-trip and marketdata p50/p99/best,
  link state, flow counters).
- `?` → the **help** overlay (every key, destructive ones flagged in red). Any
  key closes it.
