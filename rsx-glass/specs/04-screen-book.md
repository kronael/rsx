---
status: draft
---

# 04 — Screen: BOOK

BOOK is the trading screen: one symbol's order book and its history as the
log-time heatmap, with a price cursor, single-key order entry, the exact touch
ladder, the trader's position and the round-trip speed, all on one fixed grid.
It is the default screen and the one hand market-making happens on
[founder F-KEY-02 "on the book depth its for quiting"]. The heatmap encoding is
[03-heatmap.md](03-heatmap.md); the shared chrome (header, mode line, status
and hint lines, help, keycast, switcher, venue picker) is [09-chrome.md](09-chrome.md);
keys are [10-keymap.md](10-keymap.md); acceptance criteria are
[17-acceptance.md](17-acceptance.md) § BOOK.

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Layout

Reference frame — the ANSI-stripped golden, 100 × 30, live book with a
building ask wall, a burst of buy prints, an own resting buy and the cursor on
the best bid [code rsx-term/ui/testdata/book_view.golden],
[code rsx-term/ui/stream_golden_test.go TestBookViewGolden]:

```
 RSX  PENGU-PERP   ● offline  ◀ bids  mid 0.010000  asks ▶
 BOOK  rsx  RO off  PO off  size 1.0000 [1]  list rsx 
│                                                                                −24h ┆ ▲ 0.010001 
│                                                                                −24h ┆ △ 0.010001 
│                                                                                −24h ┆ △ 0.010001 
│                                                                                −24h ┆ △ 0.010001 
│                                                                                 −1h ┆ △ 0.010001 
│                                                                                 −1m ┆ △ 0.010001 
│                                                                                 −3s ┆            
│                                                                                     ┆            
│                                                                                     ┆            
│                                                                                     ┆            
│                                                                                     ┆            
│                                                                                     ┆            
│                                                                                     ┆            
│                                                                                     ┆            
│                                     ▒░░▒                                            ┆            
│                                     ▒░△▓                                            ┆            
│                                     ▒░△▓                                            ┆            
│                                     ▒░△▓                                            ┆            
│                                     ▒░△▓                                            ┆            
│                                     ▒░△█                                            ┆            
│                                     ▒░▲█                                            ┆            
│                                     ◇▁▁█                                        now ┆            
│─────────────────────────────────────▲┃┼──────────────────────────────────────      
 ask 0.010001×6.0000  0.010002×40.0000
 bid 0.009999×5.0000  0.009998×8.0000
 pos flat — fills build it
 ⚡ RTT 0 ns   p50 0 ns · p99 0 ns · best 0 ns
 x symbol  n news  b/s side  h/l cursor  j/k touch  f place  d cancel  q quit  tab view  r RO  ? help 
```

The `⚡ RTT 0 ns` line in this golden comes from a test
`Accepted` whose zero-value RTT field reads as a measured 0 ns: see BK-17.

Regions top → bottom (sizes: [03-heatmap.md](03-heatmap.md) § grid sizing):

| Region | Lines | Content |
|---|---|---|
| header | 1 | symbol badge · order-link dot · price-axis legend with the axis mid |
| mode line | 1 | screen tag · venue · RO · PO · size preset · list · capture-mode prompt |
| body | far + live + NOW | news rail · heat cells · time gutter · tape rail |
| ruler | 1 | fisheye axis · touch ticks · own orders · cursor |
| footer | 5 | ask touch ladder · bid touch ladder · position · latency · hint (or microscope) line |

Left and bottom carry the essentials so a small window still works
[founder F-SCR-01 "left and vottom would be most important things.. so that small teeminals would work too"].

## Elements

| Element | Encodes | Data source | Source |
|---|---|---|---|
| badge ` RSX  <name> ` | active instrument name, page ink on Heading | instrument config | [code rsx-term/ui/stream.go:834-839] |
| link dot | `● live` / `● offline` — the PRIMARY order link (`GwUp/GwDown`); for a standalone non-RSX venue its feed state | gateway link events | [code rsx-term/ui/stream.go:840-843], [code rsx-term/ui/update.go:76-87] |
| axis legend `◀ bids  mid <px>  asks ▶` | the heatmap's anchored centre (moves only on > 8-tick drift), `—` until anchored | `Heatmap.MidPx()` | [code rsx-term/ui/stream.go:844-851] |
| news rail | worst severity of headlines in the row's window | news source | [03-heatmap.md](03-heatmap.md) |
| heat cells | size / count / persistence / prints / own orders | per-market heatmap ring + live book | [03-heatmap.md](03-heatmap.md) |
| time gutter | `−<span>`, session-open labels, `now` | row spans and windows | [03-heatmap.md](03-heatmap.md) |
| tape rail | newest prints, one per body row, side-shaped magnitude + exact price | per-market tape (50 newest) | [code rsx-term/ui/stream.go:659-687] |
| ruler | axis, touch ticks `┼`, own `▲/▼`, cursor `┃` | own orders + per-symbol cursor | [code rsx-term/ui/stream.go:789-830] |
| ask / bid ladder | up to 3 exact levels per side `px×qty`, nearest first, `—` when empty; ask in Ask, bid in Live | live book | [code rsx-term/ui/stream.go:893-910] |
| position line | `pos flat — fills build it`; or `pos LONG/SHORT ±net @ entry   ~uPnL ±x`; Degraded `~uPnL —` + `(needs live book)` with no mid | per-market client-derived position (own fills), mark = book mid | [code rsx-term/ui/stream.go:912-947] |
| latency line | `⚡ RTT <last>   p50 · p99 · best`; before the first sample `⚡ latency: waiting for first round-trip…` | RTT window | [code rsx-term/ui/stream.go:949-959] |
| hint line | the screen's hint-flagged bindings then the globals, generated from the keymap | keymap | [code rsx-term/ui/keymap.go:279-294] |
| microscope line (replaces hint) | ` microscope <window label>  ·  ↑↓ move · enter freeze → assistant · esc off` | the selected held row | [code rsx-term/ui/stream.go:879-891] |

Per-symbol state survives a symbol hop: each market keeps its own cursor,
heatmap, position and tape [code rsx-term/ui/market.go:24-53].

## States

| State | Shows | Source |
|---|---|---|
| no window size yet / too small (< 8 heat columns or < 3 body rows) | `streaming heatmap — sizing… (waiting for terminal size / first frame)` | [code rsx-term/ui/stream.go:516-518] |
| unanchored (no two-sided book yet) | `mid —`; no cells or cursor land (no column without an anchor) | [code rsx-core/book/heatmap.go:406-409] |
| order link down | `● offline` (amber); orders fail with `submit failed: …` | [code rsx-term/ui/update.go:31-36, 618-621] |
| an empty side | `ask —` / `bid —`; `j`/`k` and `f` report `no bid to join` etc.; a cross reports `cross: no ask to lift` / `cross: no bid to hit` | [code rsx-term/ui/update.go:458-525] |
| flat | `pos flat — fills build it` | |
| no RTT yet | `⚡ latency: waiting for first round-trip…` | |
| microscope on | `▸` on the selected row, microscope line in the footer | |
| switcher / venue picker | prompt and candidates on the mode line ([09-chrome.md](09-chrome.md)) | |
| read-only venue | renders fully; every order key answers `BLOCKED: <venue> is read-only here (market data only)` | [code rsx-term/ui/update.go:394-401, 593-598] |
| over a fat-finger cap | `BLOCKED: qty <q> exceeds max (fat-finger guard)` / `BLOCKED: notional over <n> (fat-finger guard)`; nothing sent | [code rsx-term/ui/update.go:586-613] |
| keymap rejected at startup | status `KEYMAP REJECTED: <err> — defaults active` | [code rsx-term/ui/model.go:414-421] |

Status line on BOOK: at the pin the BOOK screen has no status line of its own
— its five footer lines are ladders, position, latency and hint, and status
messages (`sent …`, `BLOCKED: …`, `cancel sent …`) are written to the model
but not drawn on BOOK [code rsx-term/ui/stream.go:861-877]. NEWS, LLM and USERS
draw it. BK-10 below fixes this.

## Order entry on BOOK (game entry)

| Action | Order sent | Source |
|---|---|---|
| `f` | GTC limit, side = `b`/`s`, size = armed preset, price = cursor (else the side's own touch) | [code rsx-term/ui/update.go:476-499] |
| `⇧1`–`⇧5` | IOC at the far touch, size = that preset | [code rsx-term/ui/update.go:501-525] |
| `d` | cancel the own order nearest the cursor (else the newest) | [code rsx-term/ui/update.go:527-560] |
| RO / PO toggles | reduce-only applies to every order; post-only to GTC orders only | [code rsx-term/ui/update.go:614-617] |

Guards on every BOOK order, in order: read-only venue → qty cap → notional
ceiling. No confirm step: orders fire on one key
[founder F-KEY-01, F-KEY-02], [code rsx-term/ui/update.go:586-624].

```go
// rsx-term/ui/update.go:593-624 (exact)
func (m Model) fire(venue string, order wire.OrderReq) (tea.Model, tea.Cmd) {
	sub, whyNot := m.venueSub(venue)
	if sub == nil {
		m.status = "BLOCKED: " + whyNot
		return m, nil
	}
	ins := m.instrumentFor(venue, m.frameSymbol(order.Symbol))
	if qtyCap, ok := safeMul(maxOrderQty, pow10(ins.QtyDec)); ok && order.Qty > qtyCap {
		// The DOM cap is raw units for the primary symbol; here it scales to
		// the instrument's precision (1e6 WHOLE units) — the notional ceiling
		// below is the guard that actually bites first.
		m.status = fmt.Sprintf("BLOCKED: qty %s exceeds max (fat-finger guard)", m.fmtQty(order.Qty))
		return m, nil
	}
	if notional, ok := safeMul(order.Px, order.Qty); ok {
		ceiling, ceilOk := safeMul(m.cfg.MaxNotional, pow10(ins.PriceDec+ins.QtyDec))
		if ceilOk && notional > ceiling {
			m.status = fmt.Sprintf("BLOCKED: notional over %d (fat-finger guard)", m.cfg.MaxNotional)
			return m, nil
		}
	}
	order.ReduceOnly = order.ReduceOnly || m.reduceOnly
	if order.Tif == wire.Gtc {
		order.PostOnly = order.PostOnly || m.postOnly
	}
	if err := sub.Submit(order); err != nil {
		m.status = "submit failed: " + err.Error()
		return m, nil
	}
	m.status = m.sentStatus(order)
	return m, nil
}
```

Caps: qty cap 1,000,000 whole units at the instrument's precision; notional
ceiling `MaxNotional` human quote units per order, default 10,000
(`RSX_TERM_MAX_NOTIONAL`) [code rsx-term/ui/update.go:1156-1163]. Sent status:
`sent BUY 5.0000 @ 0.010001 [GTC] ro po` [code rsx-term/ui/update.go:1094-1105].

## Requirements beyond the terminal

| ID | Requirement | Source |
|---|---|---|
| BK-01 | Net, internal and engine legs on the BOOK latency line (not only RTT), each `—` / `·· pending` when unmeasured, with p50 and p99. | [founder F-LAT-01, F-LAT-05, F-LAT-06], [12-contract-gateway.md](12-contract-gateway.md) LAT-02/03 |
| BK-02 | The header's price label is honest: the axis centre is labelled as the axis centre; the live mid (`~mid` derived from the book) is shown separately. At the pin `mid` shows the sticky anchor, which can lag the live mid by up to 8 ticks. | [code rsx-term/ui/stream.go:844-851], [code rsx-core/book/heatmap.go:378-398], honesty H-2 |
| BK-03 | Both links are visible: the order link AND the active venue's market-data link (and their staleness, MD-11). At the pin only the order link shows; on a read-only venue the dot keeps showing the RSX order link. | [code rsx-term/ui/stream.go:840-843], [13-contract-marketdata.md](13-contract-marketdata.md) |
| BK-04 | The badge names the venue: `RSX` only on the RSX venue; another venue shows its own name (at the pin the badge reads `RSX` on every venue). | [code rsx-term/ui/stream.go:835-839] |
| BK-05 | Far rows are visibly distinct from live rows (OD-02). | [03-heatmap.md](03-heatmap.md) HM-16 |
| BK-06 | Own orders draw with one treatment on the map and the ruler (OD-07 decides whether to merge `◇` / `▲▼`). | [doc rsx-term/notes/element-inventory.md "Own-order over-encoding"] |
| BK-07 | Correlated symbols' trades appear on BOOK. | [founder F-SCR-04 "shoudl show trades on correlated symbols"]; not built at the pin; OD-08 |
| BK-08 | A per-screen key legend top-right, generated from the keymap. | [founder F-KEY-06], KM-21 |
| BK-09 | The chat key opens the assistant with the BOOK vantage (symbol, cursor, microscope row). | [founder F-AST-03], KM-20 |
| BK-10 | A status line is visible on BOOK, so every keypress's result (`sent …`, `BLOCKED: …`, `no bid to join`) is read where it happened. | [founder F-KEY-09 "as long as its clear what did happen"], [code rsx-term/ui/stream.go:861-877] |
| BK-11 | Liquidation price, leverage, available margin and ROE% join the position line when risk provides them; until then they are dashed in the derived style, never omitted silently. | [doc specs/2/55-terminal.md "New-trader safety requirements"], [founder F-HON-01] |
| BK-12 | A market-data age tag (`md <age>` / Degraded `md stale`) on BOOK. | [founder F-LAT-04], [code rsx-term/ui/view.go:829-844] (the classic view has it, BOOK does not) |
| BK-13 | Symbol and venue hops reset the microscope; the cursor is per symbol. | [code rsx-term/ui/update.go:352-388] |
| BK-14 | Sizes on the ladder and tape use the active instrument's precision, never the primary's. | [code rsx-term/ui/model.go:118-126] |
| BK-15 | The mark source is labelled: `~uPnL` is marked against the book mid; when an exchange mark feed exists (spec 15) the UI shows which mark it uses. | [doc specs/2/55-terminal.md "Data-source honesty table"] |
| BK-16 | The notional status names the instrument's quote units and the active ceiling (`BLOCKED: notional 12,400 over 10,000 (fat-finger guard)`). | [code rsx-term/ui/update.go:607-612] |
| BK-17 | An RTT of exactly 0 ns is not shown as a measurement unless the clock pair truly measured it; the client never builds an `Accepted` with an unset RTT field (a zero-valued struct must read as unknown, -1). | [code rsx-term/ui/testdata/book_view.golden], [code rsx-core/wire/order.go:9-12] |
