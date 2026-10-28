---
status: draft
---

# 05 — Screen: order entry (the classic three-column view)

The order-entry view is the terminal's original perps screen: a static price
ladder, an order form with a two-step confirm, the position with its risk
row, working orders, the public tape, the round-trip speed strip and the
latency trace — the place where typed-price orders, TIF, market, flatten,
reverse, cancel-selected, cancel-all and the confirm-off ARMED mode live. It was
the terminal's default view (the streaming screens were opt-in). Whether glass
keeps it as a separate view or folds its verbs into BOOK is OD-03 in
[16-open-decisions.md](16-open-decisions.md); every capability below is
required either way [founder F-SURF-04 "all featuews as the terminal one all of them"].
Keys: [10-keymap.md](10-keymap.md) § classic view; acceptance:
[17-acceptance.md](17-acceptance.md) § ORDER.

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Layout

Reference frame — the byte-locked golden (offline demo script folded, 120 × 32,
raw prices because the test config has 0 decimals)
[code rsx-term/ui/testdata/dom_view.golden], [code rsx-term/ui/dom_golden_test.go]:

```
 RSX  PENGU-PERP   ● live  open 0  fills 1  last 10000  ~mark 10000 (mid)  index —  funding —
┌────────────────────────────────────────────┐┌────────────────────────────────────┐┌──────────────────────────────────┐
│ book                                       ││ order                              ││ positions (mark=mid)             │
│              10011                         ││  BUY      SELL                     ││LONG  +14 @ 9998                  │
│              10010                         ││                                    ││~uPnL +28                         │
│              10009                         ││price: _                            ││liq —   ROE —   mgn ░░░░░░░░      │
│              10008                         ││qty  :                              ││(needs risk engine)               │
│              10007                         ││time-in-force: GTC                  │└──────────────────────────────────┘
│              10006                         ││reduce-only: off   post-only: off   │┌──────────────────────────────────┐
│              10005                         ││                                    ││ trades                           │
│              10004  12 ▊▊                  ││enter → preview, enter again to send││S  10000   3                      │
│              10003   8 ▊                   │└────────────────────────────────────┘│B  10001   5                      │
│              10002  20 ▊▊▊▊                │                                      └──────────────────────────────────┘
│              10001   5 ▊                   │
│‹      ▊   7  10000                         │
│     ▊▊▊  15   9999                         │
│       ▊   9   9998                         │
│  ▊▊▊▊▊▊  30   9997                         │
│               9996                         │
│               9995                         │
│               9994                         │
│               9993                         │
│               9992                         │
│               9991                         │
│               9990                         │
│               9989                         │
│████████████████████ 57% bid                │
└────────────────────────────────────────────┘
 ⚡ RTT 9.7 µs = net 2.3 µs + internal 7.1 µs + engine 310 ns  ██████████████████
   p50 10.4 µs · p99 10.4 µs · best 9.7 µs  ██▁
 order 7 done
 q quit  b/s side  t tif  r ro  p po  +/- tick  j/k join  tab field  0-9 type  ⌫ del  enter submit  m mkt  ↑↓ sel  c cancel  X all  x flatten  R reverse  F2 armed  F3 trace  ? help
```

(Trailing spaces trimmed here; the golden pads every line to the window width.)

Stack: status bar · [ARMED banner] · three columns · speed strip (2 lines) ·
status line · help legend [code rsx-term/ui/view.go:50-65]. Column content
widths: book 44, order 36, right 34; the trace HUD 46. Below 114 columns the
columns stack vertically, order form first, then book, then the right column
[code rsx-term/ui/view.go:14-25, 117-141, 172-178].

## Elements

| Element | Encodes | Data source | Source |
|---|---|---|---|
| status bar | badge; link dot; `open N  fills N`; `last <px>` (`—` before any); derived `~mark <mid> (mid)` (`~mark — (mid)` with no mid); `index —  funding —` | tape, book, order and fill counters | [code rsx-term/ui/view.go:85-115] |
| ARMED banner | ` ⚠ ARMED — confirm OFF, orders fire on one enter · F2 to re-arm safety `, Armed style, full width, persistent | `armed` flag | [code rsx-term/ui/view.go:67-75] |
| book ladder | a static price axis around a sticky centre (moves only when the mid drifts > 6 ticks); rows `mark bidBar bidQty price askQty askBar`; bars `▊` × 6 per side scaled to the deepest visible level (any nonzero ≥ 1 cell); price coloured by the resting side, Muted in the spread; `▸` own-order row (Accent), `‹` last-print row; rows per side `clamp((height−9)/2, 1, 20)` | live book, own orders, tape | [code rsx-term/ui/view.go:143-337], [code rsx-term/ui/update.go:804-821] |
| imbalance bar | 20-cell bid|ask split of visible depth + `NN% bid` | live book | [code rsx-term/ui/view.go:257-282] |
| degraded book | one Degraded row `no live book — market-data stream down` when the book is empty OR the md link is down | md link + book | [code rsx-term/ui/view.go:205-208] |
| order form | `  BUY  ` / `  SELL  ` (active side reversed); `price:` / `qty:` fields (focused bold bright with `_`); `time-in-force: GTC|IOC|FOK`; `reduce-only: on/off   post-only: on/off`; tail `enter → preview, enter again to send` | form state | [code rsx-term/ui/view.go:423-453] |
| confirm preview (replaces the tail) | Ring divider `── confirm ──`; `<SIDE> <qty> @ <px>` side-hued; `notional <n>  <TIF>` (notional `—` on overflow); `ro:on/off  po:on/off   liq n/a`; Heading bold `enter again to SEND` + ` · esc` | pending order | [code rsx-term/ui/view.go:462-486] |
| positions panel | title `positions (mark=mid)`; flat: `no position — fills build it`; open: `LONG/SHORT  ±net @ entry`, then `~uPnL ±x` (or Degraded `~uPnL —` + `(needs live book)`), then the risk row | client-derived position | [code rsx-term/ui/view.go:495-540] |
| risk row | `liq —   ROE —   mgn ░░░░░░░░` and `(needs risk engine)`, all derived style — the fixed home of the risk surface, never a number | none yet | [code rsx-term/ui/view.go:542-559] |
| working orders (only when non-empty) | header `side  px  qty`; rows `▸ BUY  <px> <qty>`; `▸` marks what `c` cancels | own orders | [code rsx-term/ui/view.go:395-421] |
| trades | newest first, `B`/`S` + px + qty in the taker hue; `no trades yet` | tape | [code rsx-term/ui/view.go:561-591] |
| speed strip line 1 | ` ⚡ RTT <total> ` bold, Heading ≤ 50 µs, Degraded > 50 µs, Ask > 100 µs; `= net <x> + internal <y> + engine <z>` (a leg the server does not stamp: `·· pending` derived style); an 18-cell proportional hop bar of the KNOWN legs (net Heading, internal Accent, engine Live) | last RTT sample | [code rsx-term/ui/view.go:742-827] |
| speed strip line 2 | `p50 · p99 · best` (Muted) + a 32-sample sparkline (Accent) + `· md <age>` or Degraded `· md stale <age>` (> 2 s since the last md frame) | RTT window, md age | [code rsx-term/ui/view.go:820-844] |
| speed strip before any RTT | ` ⚡ latency: waiting for first round-trip…` | | [code rsx-term/ui/view.go:799-802] |
| status line | the last event, TextBright | model status | [code rsx-term/ui/view.go:846-848] |
| help legend | the static legend string | | [code rsx-term/ui/view.go:38-40] |

```go
// rsx-term/ui/sparkline.go:3-35 (exact, comments trimmed)
var sparkRamp = []rune("▁▂▃▄▅▆▇█")

func sparkline(samples []int64) string {
	if len(samples) == 0 {
		return ""
	}
	lo, hi := samples[0], samples[0]
	for _, v := range samples[1:] {
		if v < lo {
			lo = v
		}
		if v > hi {
			hi = v
		}
	}
	span := hi - lo
	out := make([]rune, len(samples))
	for i, v := range samples {
		if span == 0 {
			out[i] = sparkRamp[len(sparkRamp)/2]
			continue
		}
		idx := (v - lo) * int64(len(sparkRamp)-1) / span
		out[i] = sparkRamp[idx]
	}
	return string(out)
}
```

## Overlays on this view

| Overlay | Content | Source |
|---|---|---|
| trace HUD (`F3`, swaps the right column) | Ring panel `TRACE — F3 to hide`; LINKS (`gw connected|down <endpoint>`, `md …`); LATENCY round-trip (net / internal / engine legs, `p50 … p99 … best …`, sample count, recent sparkline or `(no samples yet)`); LATENCY marketdata (`age` or `no frame yet`, p50/p99/best, `since <age> ago` or Degraded `… — STALE`); FLOW (open, fills, spread, depth `N bid / M ask`, last status); a pending-leg legend naming the frames that carry zero stamps (the inserted and failed `U`) while legs are unstamped | [code rsx-term/ui/view.go:593-740] |
| help (`?`) | Ring panel `KEYS — ? or any key to close`, grouped: order entry · orders & position · view; destructive keys (`c`, `X`, `x`, `R`, `F2`, `q`) in Ask | [code rsx-term/ui/view.go:354-393] |

The trace HUD is the video-game overlay the founder asked for
[founder F-LAT-02 "like an overdraw over a video game ... using some fxxx shortcut"].

## Order paths (all through one gate)

```go
// rsx-term/ui/update.go:1170-1186 (exact)
func (m Model) arm(order wire.OrderReq) (tea.Model, tea.Cmd) {
	if order.Qty > maxOrderQty {
		m.status = fmt.Sprintf("BLOCKED: qty %s exceeds max %s (fat-finger guard)", m.fmtQty(order.Qty), m.fmtQty(maxOrderQty))
		return m, nil
	}
	if m.armed {
		if err := m.cfg.Sub.Submit(order); err != nil {
			m.status = "submit failed: " + err.Error()
			return m, nil
		}
		m.status = m.sentStatus(order)
		m.clearForm()
		return m, nil
	}
	m.pendingConfirm = &order
	return m, nil
}
```

| Path | Builds | Refusals (status) | Source |
|---|---|---|---|
| `enter` (form) | limit at the typed price and qty, form side / TIF / RO / PO | `incomplete order (need price & qty)` | [code rsx-term/ui/update.go:1044-1092] |
| `m` market | IOC at the far touch for the typed qty, RO from the form | `market: enter a qty first`, `market: no ask to cross` / `no bid to cross` | [code rsx-term/ui/update.go:1128-1154] |
| `x` flatten | reduce-only IOC of `|net|` at the opposing touch | `no position to flatten`, `can't flatten: no live book to price against` | [code rsx-term/ui/update.go:1107-1126, 1220-1247] |
| `R` reverse | IOC of `2 × |net|` on the opposite side at the far touch, NOT reduce-only | `no position to reverse`, `reverse: no ask to cross` / `no bid to cross` | [code rsx-term/ui/update.go:1188-1218] |

Second `enter` submits the pending order; a failed submit keeps the preview so
a retry-enter resubmits; success clears price, qty, RO, PO and focus but keeps
side and TIF [code rsx-term/ui/update.go:1053-1074]. `esc` with a preview
cancels it (`order not sent`). Any editing key clears the preview. `F2`
clears any preview. Cancel: `c` the selected order (`cancel sent for order N`,
`no open order to cancel`, `selected order has no cid yet`, `cancel failed: …`);
`X` every order with a cid (`cancel-all sent (N orders)`, `no open orders to cancel`)
[code rsx-term/ui/update.go:1259-1299]. The qty cap on this path is
1,000,000 RAW units (lots), not scaled by precision — unlike BOOK's
[code rsx-term/ui/update.go:1156-1159, 600-605].

Decimal input: the trader types the human decimal they read off the ladder
and the raw i64 is reconstructed; more fractional digits than the instrument
carries, a second dot or a non-digit is invalid, never silently rounded:

```go
// rsx-term/ui/format.go:52-76 (exact)
func parseRaw(s string, dec int) (int64, bool) {
	if s == "" {
		return 0, false
	}
	intPart, fracPart := s, ""
	if i := strings.IndexByte(s, '.'); i >= 0 {
		intPart, fracPart = s[:i], s[i+1:]
	}
	if strings.IndexByte(fracPart, '.') >= 0 { // a second dot
		return 0, false
	}
	if dec <= 0 {
		if fracPart != "" {
			return 0, false // no fractional part at zero decimals
		}
		return parseDigits(intPart)
	}
	if len(fracPart) > dec {
		return 0, false // more precision than the instrument carries
	}
	for len(fracPart) < dec {
		fracPart += "0"
	}
	return parseDigits(intPart + fracPart)
}
```

## Requirements beyond the terminal

| ID | Requirement | Source |
|---|---|---|
| OE-01 | The confirm preview shows the resulting liquidation price when risk provides it; until then `liq n/a` (or `—`), never omitted. | [doc specs/2/55-terminal.md "Order lifecycle & confirmation"], [founder F-HON-01] |
| OE-02 | New-trader safety surface, ranked: liquidation price; unmistakable side (colour + LONG/SHORT word); uPnL in $ and ROE%; available margin; confirm-before-submit; leverage next to size; mark vs last both labelled. Unsourced fields are dashed, never faked. | [doc specs/2/55-terminal.md "New-trader safety requirements"], [founder F-HON-01] |
| OE-03 | Working-order rows show remaining qty after partial fills and are keyed by oid. | GW-25, GW-26 in [12-contract-gateway.md](12-contract-gateway.md) |
| OE-04 | Qty is lot-aligned before submit (`E[1009]` otherwise). | MD-23 |
| OE-05 | The trace HUD's pending legend names the real reason a leg is pending, from the current state of the server stamps (LAT-04), not hard-coded bug ids. | [code rsx-term/ui/view.go:726-729] |
| OE-06 | `?` help on this view is generated from the one keymap table (KM-22). | |
| OE-07 | Leverage and margin-mode selectors: display-only until risk exposes them; shown dashed. | [doc specs/2/55-terminal.md "order"] |
| OE-08 | Multi-symbol: this view trades the active instrument and formats with its precision (at the pin it formats notional with the primary's precision). | [code rsx-term/ui/model.go:173-186] |
