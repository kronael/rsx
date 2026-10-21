---
status: draft
---

# 11 — Flows

A flow is one whole user journey — the keys pressed, the events the exchange
sends back, what the trader sees — and every flow here is backed by a test
that drives the model through it, so the document and the executable check
stay in lockstep [founder F-TST-03 "are flows documented?  in FLOWS.md ?"].
Glass ports each flow as a test of its own model (and, where the stack is
needed, an end-to-end test against the real cluster, [17-acceptance.md](17-acceptance.md)).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.
Test names are `rsx-term/ui/<file>::<Test>` at the pin.

## Order-entry view flows (FLOWS.md 1-15)

PENGU-PERP at price 6 dp, qty 4 dp, tick 1: the trader reads and types
`0.010001`; the wire carries `10001`. Test setup [code rsx-term/ui/flows_test.go:19-37]:

```go
// rsx-term/ui/flows_test.go:21-37 (exact)
func pengu(mock *conn.MockGateway) Model {
	return New(Config{Symbol: "PENGU-PERP", SymbolID: 10, Sub: mock, PriceDec: 6, QtyDec: 4, Tick: 1})
}

func live(mock *conn.MockGateway) Model {
	m := pengu(mock)
	m = apply(m, feed.GwUp{})
	m = apply(m, feed.MdUp{})
	m = apply(m, wire.Snapshot{
		Bids: []wire.Level{{Px: 10000, Qty: 70000}, {Px: 9999, Qty: 150000}},
		Asks: []wire.Level{{Px: 10002, Qty: 60000}, {Px: 10003, Qty: 250000}},
	})
	m.width, m.height = 120, 26
	return m
}
```

| # | Flow | Steps → expected | Test |
|---|---|---|---|
| 1 | Startup and connect | before md: book shows `no live book`, dot offline → `GwUp`: status `connected` → `MdUp` + snapshot: ladder with decimal prices | `flows_test.go::TestFlowStartupConnect` |
| 2 | Place a limit | type `0.010001`, `tab`, `5`, `enter` → confirm preview, nothing sent → `enter` → `N` with px 10001 qty 50000 GTC; status `sent BUY 5.0000 @ 0.010001 [GTC]` → `Accepted` → working order listed, ladder row marked `▸`. `esc` at the preview cancels | `flows_test.go::TestFlowPlaceLimitOrder` |
| 3 | Cancel the selected order | two working orders, `down`, `c` → cancel by the SECOND order's cid → `Done` removes only it | `flows_test.go::TestFlowCancelSelected` |
| 4 | Cancel all | `X` → a cancel per working order | `flows_test.go::TestFlowCancelAll` |
| 5 | Market order | `tab`, `3`, `m` → preview IOC at the far touch (10002 for a buy) → `enter` → IOC sent | `flows_test.go::TestFlowMarketOrder` |
| 6 | A fill builds a position | `Fill` 15 @ 0.009999 → `LONG +15.0000`, `~uPnL`, fills counter 1; risk row stays dashed | `flows_test.go::TestFlowFillBuildsPosition` |
| 7 | Flatten | long 15, `x` → preview reduce-only SELL 15 @ best bid → `enter` → sent. Flat or bookless: a no-op with a reason | `flows_test.go::TestFlowFlatten` |
| 8 | Reverse | long 15, `R` → preview SELL 30, NOT reduce-only | `flows_test.go::TestFlowReverse` |
| 9 | ARMED | `F2` → banner; one `enter` sends; an oversized order is still `BLOCKED`; `F2` again restores the confirm | `flows_test.go::TestFlowArmedConfirmOff` |
| 10 | Price helpers | `j` → `0.010000`, `+` → `0.010001`, `k` → `0.010002` | `flows_test.go::TestFlowPriceHelpers` |
| 11 | Fat-finger block | qty `999999` lots → `enter` → no preview, nothing sent, status `BLOCKED: …` | `flows_test.go::TestFlowFatFingerBlock` |
| 12 | Reject | `Rejected{insufficient margin}` → status `rejected: insufficient margin` | `flows_test.go::TestFlowReject` |
| 13 | Market data down and back | `MdDown` → `no live book` row → `MdUp` + a BBO → ladder restored | `flows_test.go::TestFlowMarketdataDegraded` |
| 14 | Gateway down | `GwDown` → `● offline`; md stays up | `flows_test.go::TestFlowGatewayDegraded` |
| 15 | Overlays | `F3` → trace with LATENCY; `?` → help with KEYS; any key closes help | `flows_test.go::TestFlowOverlays` |

```go
// rsx-term/ui/flows_test.go:218-244 (exact) — flow 9, the guard outlives the confirm
func TestFlowArmedConfirmOff(t *testing.T) {
	mock := &conn.MockGateway{}
	m := live(mock)
	m = press(m, "f2") // ARM
	if !strings.Contains(stripANSI(m.viewArmedBanner()), "ARMED") {
		t.Fatal("armed banner should warn")
	}
	m = typeDigits(m, "0.010001")
	m = press(m, "tab")
	m = typeDigits(m, "5")
	m = press(m, "enter") // single enter fires
	if len(mock.Submitted) != 1 || m.pendingConfirm != nil {
		t.Fatalf("ARMED single-enter should submit without a preview: submitted=%d", len(mock.Submitted))
	}
	// Even ARMED, an oversized order is hard-blocked — ARMED removes the
	// confirm, never the fat-finger guard.
	blocked := live(mock)
	blocked = press(blocked, "f2")
	blocked = typeDigits(blocked, "0.010001")
	blocked = press(blocked, "tab")
	blocked = typeDigits(blocked, "999999") // way over the cap
	before := len(mock.Submitted)
	blocked = press(blocked, "enter")
	if len(mock.Submitted) != before || !strings.Contains(blocked.status, "BLOCKED") {
		t.Fatalf("fat-finger guard must hold in ARMED: status=%q", blocked.status)
	}
}
```

## Streaming flows

Setup: 100 × 30, PENGU two levels a side, one bin sealed so the axis is
anchored [code rsx-term/ui/stream_test.go:17-48].

| # | Flow | Steps → expected | Test |
|---|---|---|---|
| S1 | Preset and place | `3` (5 units), `f` with no cursor → GTC BUY 50000 @ best bid 9999 | `stream_test.go::TestGameEntryPresetAndPlace` |
| S2 | Cursor and place | `j` (9999), `h`, `h` → `f` → px 9997 | `stream_test.go::TestGameEntryCursorMovesAndPlaces` |
| S3 | Cross | `s`, `@` → IOC SELL 20000 @ best bid 9999 | `stream_test.go::TestGameEntryCrossIsIocAtFarTouch` |
| S4 | Caps hold on one key | preset over the qty cap → `f` → nothing sent, `BLOCKED`; notional ceiling 1 → `f` → `BLOCKED` | `stream_test.go::TestGameEntryFatFingerStillBlocks`, `watchlist_test.go::TestBookNotionalCapBlocks` |
| S5 | Cancel nearest | own buys at 9995 and 9999, `j`, `d` → cancels the 9999 order | `stream_test.go::TestGameEntryCancelNearestCursor` |
| S6 | RO toggle applies | `r`, `f` → the order is reduce-only; ` RO ` on the mode line | `watchlist_test.go::TestBookReduceOnlyToggleApplies` |
| S7 | Own orders visible | an `Accepted` buy at 9998 → `◇` on the NOW row and `▲` on the ruler | `stream_test.go::TestStreamOwnOrdersMarkedOnMapAndRuler` |
| S8 | Microscope | `up` enters at the newest held row; repeated `up` clamps at the oldest; `down` steps newer; `esc` turns it off without quitting | `microscope_test.go::TestMicroscopeEnterMoveExit` |
| S9 | Honest freeze label | walking into far rows shows `aggregate, not a restored book`; the newest row `exact ~100ms bin` | `microscope_test.go::TestMicroscopeFarRowIsHonestWindow`, `…::TestMicroscopeNewestRowIsExactBin` |
| S10 | Freeze to assistant | `up`, `enter` → LLM screen; context origin book freeze, no headline, a note; the pane shows `book freeze`, the label and the placeholder, never `severity` | `microscope_test.go::TestMicroscopeFreezeHandsRowToAssistant`, `…::TestBookFreezeAssistantRendersOriginNote` |
| S11 | Symbol hop | `x` + WIF's code → the book shows WIF-PERP and its touch `0.021010` instantly; the microscope resets | `watchlist_test.go::TestBookSymbolSwitcher`, `microscope_test.go::TestMicroscopeSwitchSymbolResetsCursor` |
| S12 | Per-symbol positions | fills on symbols 10 and 11 → +100 and −50, independent | `watchlist_test.go::TestPerSymbolPositionsIndependent` |
| S13 | Venue hop | `F9`, `h` → Hyperliquid's first instrument (BTC) on BOOK | `venue_test.go::TestVenuePickerSwitchesBook` |
| S14 | Read-only venue | on Hyperliquid with a book, `!` → nothing sent, status mentions `read-only` | `venue_test.go::TestReadOnlyVenueBlocksOrders` |
| S15 | Venue frames isolated | a tagged HL snapshot lands in the HL market only, never the RSX or classic book | `venue_test.go::TestVenueMsgRoutesToItsOwnMarket` |
| S16 | Screen cycle | `tab` × 4: NEWS, LLM, USERS, BOOK; `shift+tab` → USERS | `watchlist_test.go::TestScreenCycle` |
| S17 | News search | `/`, type → feed filters, `q` types (does not quit), `esc` clears | `news_view_test.go::TestNewsSearchFilters`, `keymap_test.go::TestSearchCapturesGlobalKeys` |
| S18 | News handoff | select a SOL headline, `enter` → LLM; context links SOL's market, freezes its book and mid; later book changes do not alter the context | `news_view_test.go::TestNewsSelectionAndHandoff` |
| S19 | Letter jump | a symbol's code on NEWS → BOOK on that symbol | `news_view_test.go::TestNewsLetterJumpsToBook` |
| S20 | `n` from BOOK | → NEWS | `news_view_test.go::TestBookNKeyOpensNews` |
| S21 | Live chat | handoff posts the context turn, opens a busy thread (`t-<ms>-<venue>-<symbol>`), the streamed reply renders; typing extends the draft; `enter` posts on the same thread and echoes `you`; `enter` while busy keeps the draft and explains; a stale thread's reply is dropped; a failure renders `assistant unreachable` and clears busy | `news_view_test.go::TestLLMLiveHandoffStreamsReply`, `…::TestLLMLiveTypedFollowUp`, `…::TestLLMBusyGatesFollowUp`, `…::TestLLMIgnoresStaleThreadReply`, `…::TestLLMFailureRendersUnreachable` |
| S22 | Users | list shows fetched users and the caller's role; `/bob` filters; `enter` → detail with account and position; `a 500 T-1 fix` `enter` → `adjust-collateral +500 ok` and the fixture reflects it; a malformed prompt stays open with the reason; `esc` backs out one layer at a time to BOOK | `users_view_test.go::TestUsersListShowsFetchedUsers` … `::TestUsersEscBacksOutOneLayerAtATime` |
| S23 | Rebind | `{"place":"o"}` → `o` places, `f` does nothing, hint shows `o place`; an unknown action or a collision → `KEYMAP REJECTED`, defaults active | `keymap_test.go::TestKeymapOverrideDrivesDispatch`, `…::TestKeymapBrokenOverridesFallBackLoudly` |
| S24 | Keycast | `f` → chip `⌨ f  place resting limit at the cursor`; `up` → `⌨ ↑` without the parenthetical; `3` and `!` labelled from the class docs; `x` then `s` → bare `⌨ s`; off → frame unchanged; 20 ticks → gone | `keycast_test.go::*` (6 tests) |
| S25 | Help per screen | `?` on BOOK → `KEYS — BOOK` with `place resting limit`, `⇧1-5`, the rebind note; any key closes; on NEWS → `KEYS — NEWS` with `hand the headline` | `stream_test.go::TestStreamHelpOverlay` |

```go
// rsx-term/ui/stream_test.go:232-244 (exact) — S3
func TestGameEntryCrossIsIocAtFarTouch(t *testing.T) {
	mock := &conn.MockGateway{}
	m := streamModel(t, mock)
	m = press(m, "s") // sell side
	m = press(m, "@") // shift+2: cross with preset 2
	if len(mock.Submitted) != 1 {
		t.Fatalf("cross should fire one order, got %d", len(mock.Submitted))
	}
	order := mock.Submitted[0]
	if order.Side != wire.Sell || order.Px != 9999 || order.Qty != 20000 || order.Tif != wire.Ioc {
		t.Fatalf("cross = %+v, want SELL 20000 @ 9999 IOC", order)
	}
}
```

## Connection flows (transport tests)

| # | Flow | Expected | Test |
|---|---|---|---|
| C1 | Frames fold | in-process WS servers for gateway and md: the frame → message path matches the offline script | `rsx-term/conn/live_test.go::TestLiveGatewayFoldsFrames` |
| C2 | Order link drop | `GwDown`, a redial with a FRESH JWT, `GwUp` | `…::TestLiveGatewayReconnectsGwWithBackoff` |
| C3 | Md link drop | `MdDown`, redial, the subscribe frame re-sent, `MdUp` | `…::TestLiveGatewayReconnectsMdAndResubscribes` |
| C4 | Close wins | Close during a reconnect backoff stops the reader | `…::TestLiveGatewayCloseStopsReconnect` |
| C5 | Submit before connect | errors, no panic | `…::TestLiveGatewaySubmitBeforeConnectErrors` |
| C6 | Multi-symbol | one `S` frame per watched symbol; `Symbol` routes the `N` frame | `…::TestLiveGatewayWatchSymbolsAndRouting` |
| C7 | Md down at start | the initial md dial failure retries and recovers | `…::TestLiveGatewayMdInitialDialFailureRetries` |
| C8 | Demo script | folding the whole offline script ends LONG 14 @ 9998 | `rsx-term/conn/mock_test.go::TestDemoScriptFolds` |
| C9 | Live Hyperliquid | opt-in: real meta, subscribe, folds, a render of the screens | `rsx-term/hl_smoke_test.go::TestHLStandaloneSmoke` |
| C10 | Live assistant | opt-in: one real chat turn per coin (SOL, ETH, XRP) returns an on-topic reply | `rsx-term/assistant/smoke_test.go::TestLLMSmoke` |

## The demo walk

The recorded tour drives the real UI on a live cluster staged for drama (a
walls-profile book, a +45% pump with a real risk-engine liquidation, a demo
position ending ≈ +10k), keycast on, in this order
[code rsx-term/demo/walk.sh], [founder F-DEM-02]:

1. BOOK streams; `j`, `l`, `l`, `k`, `h` move the cursor while the demo user's
   entry fills fold into the position.
2. During the pump: `k`, `b`, `j`, `f` (a resting limit `◇`), `d` (cancel),
   `k`, `r` `r`, `p` `p` (modifier blips).
3. `up` × 3 over the move's history, `enter` freezes → assistant; `tab` back,
   `esc` microscope off.
4. `n` NEWS, `/btc`, `esc`.
5. `tab` to LLM: wait for the real reply (`⏳` clears), type a question,
   `enter`, wait for the second reply.
6. `tab`, `F9`, `h` (live Hyperliquid BTC), `F9`, `r`, `?`, `esc`.
7. `j`, hold on the position line and the cascade bands, `q`.

No IOC cross is in the tour: the demo cluster's completion path wedged minutes
after boot [BUGS RISK-CMP-DESYNC-ORDER-EVENT-STARVE],
[doc rsx-term/demo/CLAUDE.md].
