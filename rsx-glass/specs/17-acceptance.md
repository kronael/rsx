---
status: draft
---

# 17 — Acceptance criteria

These are the checks a glass implementation passes to count as having absorbed
the terminal: one row per criterion, grouped by screen, each naming the
terminal test it is ported from (or the requirement it newly enforces) and the
kind of glass test that proves it. Kinds: **model** — drives the UI model with
messages and keys, asserts state and status text, no pixels; **golden** —
renders a fixed scene to a byte-locked PNG through the `soft` renderer;
**matrix** — a colour/contrast computation over every ink × background ×
theme; **connector** — the transport against in-process servers or `mockx`;
**e2e** — the real binary against a real local cluster, measuring speed as it
goes [founder F-TST-01, F-TST-02, F-TST-04].

Source pin: terminal tests are `rsx-term/<path>::<Test>` at
`5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Rules for the suite

| ID | Rule | Source |
|---|---|---|
| AC-00 | Every flow in [11-flows.md](11-flows.md) is one test with the same number; the doc and the test change together. | [founder F-TST-03] |
| AC-01 | Every screen has a golden per state listed in its file (default, empty, degraded, offline, each capture mode). A golden drift fails; a re-bless is reviewed, never automatic. | [doc rsx-glass/CLAUDE.md "The soft golden is a REGRESSION oracle"] |
| AC-02 | Colour is checked by the matrix test, never by goldens alone. | DS-14 |
| AC-03 | The offline default makes zero network calls: a test fails if any dial site runs without its opt-in. | [code rsx-term/news/treeofalpha_test.go TestDialSiteIsOnlyInStart], [code rsx-term/support/http_test.go TestOfflineDefaultsNeverDial] |
| AC-04 | The render path allocates nothing per frame after warm-up. | [doc rsx-glass/CLAUDE.md "Alloc-free per frame"] |
| AC-05 | Full-stack e2e flows run the binary against `make demo` and record the order round-trip p50 and p99 from the same run; the UI's speed readout is a demo aid, never the exchange's latency claim. | [founder F-TST-02, F-LAT-05], [doc CLAUDE.md "Latency & performance verification"] |

## Design system

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-DS-01 | Reserved glyph ink ≥ 3:1 on every ramp tier; overlay inks ≥ 3:1 on tiers 0–1; text inks ≥ 3:1 on the page; both themes. | matrix | `ui/palette_test.go::TestPaletteContrast` |
| AC-DS-02 | The colourblind theme changes only bid/ask (and their derivatives); every other token is identical. | model | `ui/orders_test.go::TestUseThemeColorblind` |
| AC-DS-03 | Every renderable shape is in the design-system legend ([02-design-system.md](02-design-system.md)); no braille codepoint is used. | model | `ui/glyphs_test.go::TestGlyphVocabularyDocumented`, `::TestGlyphVocabularyExcludesBraille` |
| AC-DS-04 | A desaturated render keeps buy and sell prints distinct and keeps their magnitude order. | golden | `ui/stream_test.go::TestCellTradeSideByShape` |
| AC-DS-05 | Number formatting: `fmtDec(10001,6)=0.010001`, `fmtDec(100000,4)=10.0000`, negatives signed, 0 decimals raw; `fmtNs` ladder; money at quote precision (`0.050005`, `-0.000250`). | model | `ui/orders_test.go::TestFmtDec`, `::TestFmtNotionalQuotePrecision` |
| AC-DS-06 | An overflowing notional renders `—`, never a wrapped number. | model | `ui/orders_test.go::TestSafeMulGuardsNotional` |

## Heatmap

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-HM-01 | The frame is exactly the window height whether the ring is empty or full. | model | `ui/stream_test.go::TestStreamFixedGrid` |
| AC-HM-02 | The gutter shows `−3s`, `−1m`, `−1h`, `−24h`, `now`. | model | `ui/stream_test.go::TestStreamGutterShowsHorizons` |
| AC-HM-03 | A ≥ 1 h row whose window spans 08:00 UTC shows `EU`; a 3 s row keeps `3s`. | model | `ui/stream_test.go::TestGutterLabelsSessionOpen` |
| AC-HM-04 | Size tier: 0 for 0; ≥ 1 for any size; the reference max hits the top tier. | model | `ui/stream_test.go::TestSizeTierLogScaled` |
| AC-HM-05 | Count channel: one big order shows step 1, twenty small orders step 4. | model | `ui/stream_test.go::TestCellCountChannel` |
| AC-HM-06 | Persistence mark at exactly 30 s, not at 30 s − 1 ns. | model | `ui/stream_test.go::TestCellPersistenceGlyph` |
| AC-HM-07 | A print overrides the book cell; a bigger print picks a heavier mark; a buy uses the buy ramp. | model | `ui/stream_test.go::TestCellTradeLayerCoEqual` |
| AC-HM-08 | The stable basis jumps to a new max, decays by 1/256 per bin, floors at 1. | model | `ui/stream_test.go::TestStableBasisNeverFlickersDown` |
| AC-HM-09 | Fisheye: `FisheyePx(FisheyeCol(p))` returns the inner edge of p's bucket for odd and even anchors; an absurd price clamps to the edge in bounded time. | model | `rsx-core/book/heatmap_test.go::TestFisheyePxRoundTrip`, `::TestFisheyeColAbsurdPxStaysBounded`, `::TestCompressTicksBoundedOnHugeExtra` (survive) |
| AC-HM-10 | Cascade: a bin wider than a tier's span is split, no time dropped, trades counted once. | model | `rsx-core/book/heatmap_test.go::TestCascadeSplitsOvershootAcrossTiers`, `::TestCascadeSealsFirstTierWindow` (survive) |
| AC-HM-11 | Far rows render visibly distinct from live rows (OD-02 treatment). | golden | new (HM-16) |

## BOOK

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-BOOK-01 | Reference scene (wall builds, buy burst, own buy, cursor on best bid) matches its golden. | golden | `ui/stream_golden_test.go::TestBookViewGolden` |
| AC-BOOK-02 | Touch ladder shows `0.009999×5.0000`, `0.009998×8.0000`, `0.010001×6.0000` for the standard scene. | model | `ui/stream_test.go::TestStreamFooterTouchLadder` |
| AC-BOOK-03 | Flows S1–S7 pass. | model | `ui/stream_test.go::TestGameEntry*`, `ui/watchlist_test.go::TestBook*` |
| AC-BOOK-04 | Tape rail lists a print with its exact price. | model | `ui/stream_test.go::TestStreamTapeRailShowsPrints` |
| AC-BOOK-05 | Microscope flows S8–S10 pass, including the honest labels. | model | `ui/microscope_test.go::*` |
| AC-BOOK-06 | Before sizing: the sizing message; before a two-sided book: `mid —`; before an RTT: the waiting line. | model | `ui/stream.go` states (new tests) |
| AC-BOOK-07 | The latency line shows net / internal / engine from stamps when present, `·· pending` when zero (BK-01, LAT-03/04). | model | new |
| AC-BOOK-08 | A status line shows every keypress's result on BOOK (BK-10). | model | new |
| AC-BOOK-09 | The header distinguishes the axis centre from the live mid (BK-02); shows the active venue (BK-04); shows both links (BK-03). | golden | new |

## Order entry

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-ORDER-01 | Reference scene (the offline demo script folded) matches its golden. | golden | `ui/dom_golden_test.go::TestDomViewGolden` |
| AC-ORDER-02 | Flows 1–15 pass. | model | `ui/flows_test.go::TestFlow*` |
| AC-ORDER-03 | Form editing: digits land in the focused buffer; 18-char cap; one dot; leading dot → `0.`; ignored at 0 dp; backspace; tab keeps the other buffer. | model | `ui/update_test.go::TestTypingLandsInFocusedBuffer`, `::TestDigitCap`, `::TestBackspace`, `ui/orders_test.go::TestAppendDotRules` |
| AC-ORDER-04 | `parseRaw` table: `"0.010001"@6 → 10001`, `"5"@4 → 50000`, `".5"@4 → 5000`, too many decimals / second dot / non-digit → invalid. | model | `ui/orders_test.go::TestParseRaw`, `::TestDecimalInputSubmitsRaw` |
| AC-ORDER-05 | Confirm gate: one enter never submits; esc cancels with `order not sent`; an edit clears the preview; a failed submit keeps it for retry; success clears px/qty/ro/po and keeps side/tif. | model | `ui/update_test.go::TestSingleEnterSubmitsNothing`, `::TestEscClearsPendingConfirm`, `::TestEditKeyClearsPendingConfirm`, `::TestSubmitAgainstDownMock`, `::TestSubmitGtcBuy`, `::TestSubmitIocSellReducePost` |
| AC-ORDER-06 | Flatten builds reduce-only IOC at the opposing touch; flat or bookless → a reason, no preview. | model | `ui/update_test.go::TestBuildFlattenOrder*`, `::TestFlattenKey*` |
| AC-ORDER-07 | Ladder: rows show qty/price/bar; bars 6/3/1/0 cells for full/half/tiny/zero; the axis does not move on small drift, re-centres on large. | model | `ui/ladder_test.go::*` |
| AC-ORDER-08 | Own-order `▸` and last-print `‹` rows; working-order panel; cancel targets the selection. | model | `ui/orders_test.go::TestLadderMarkers`, `::TestViewOpenOrders`, `::TestCancelSelected`, `::TestCancelAll` |
| AC-ORDER-09 | Fat-finger: blocked in every path including ARMED. | model | `ui/orders_test.go::TestFatFingerGuard`, `::TestArmedStillHonorsFatFingerGuard` |
| AC-ORDER-10 | Risk row is dashed in the derived style, no number. | model | `ui/orders_test.go::TestRiskRowIsHonestlyDashed` |
| AC-ORDER-11 | Below 114 columns the panels stack; at 132 they sit side by side; a tiny window does not crash. | model | `ui/orders_test.go::TestNarrowStacksPanels`, `ui/view_test.go::TestViewTinyWindowDoesNotPanic` |
| AC-ORDER-12 | Speed strip: waiting line, then RTT; hop bar renders only known legs; sparkline scales to its own range, flat → mid glyph. | model | `ui/view_test.go::TestViewWaitingBeforeLatency`, `::TestViewRttAfterLatency`, `ui/orders_test.go::TestHopBar`, `ui/sparkline_test.go::*` |
| AC-ORDER-13 | Trace HUD opens on F3 and shows LATENCY. | model | `ui/view_test.go::TestViewTrace` |

## NEWS

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-NEWS-01 | Sector map and feed render; the screen is the fixed height. | model + golden | `ui/news_view_test.go::TestNewsViewSectorMapAndFeed`, `ui/stream_golden_test.go::TestNewsViewFixedShape` |
| AC-NEWS-02 | Co-move: identical series +1, opposite −1, short series no figure, zig-zag near 0; reference BTC then ETH; SOL rising with BTC reads `≡`/`=`, falling against it `≠`. | model | `ui/comove_test.go::*` |
| AC-NEWS-03 | Flows S17–S20 pass. | model | `ui/news_view_test.go::*`, `ui/keymap_test.go::TestSearchCapturesGlobalKeys` |
| AC-NEWS-04 | A small negative move keeps its minus (NW-02); the tile names its reference window (NW-01). | model | new |

## LLM

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-LLM-01 | Offline: the placeholder lines are byte-exact, the header says no model is wired, no live chrome, busy never set. | model | `ui/news_view_test.go::TestAssistReplyLinesOfflineExact`, `::TestLLMOfflinePaneUnchanged` |
| AC-LLM-02 | Handoff context renders; esc returns to NEWS; no context → the instruction line. | model | `ui/news_view_test.go::TestLLMViewRendersHandoffAndPlaceholder`, `::TestLLMViewWithoutContext` |
| AC-LLM-03 | Flow S21 passes against a local SSE server. | connector | `ui/news_view_test.go::TestLLMLive*`, `assistant/client_test.go::*` |
| AC-LLM-04 | Prompt render: news block, position and orders, dashes for unknowns. | model | `assistant/prompt_test.go::*` |
| AC-LLM-05 | Handoff packaging: news carries the headline and deep-copies levels; a book freeze has no headline. | model | `news/context_test.go::*` |
| AC-LLM-06 | AS-01 … AS-05 (chat key, vantage, MCP read and control, no agent execution). | model + connector | new |

## USERS

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-USERS-01 | Offline by default with the honest line; list with the caller's role; search filters; detail with account and position; offline accounts honest; adjust-collateral round trip; malformed prompt stays open; esc one layer at a time. | model | `ui/users_view_test.go::*` (8 tests) |
| AC-USERS-02 | Roles client: disabled without URL or token; round trip; a 401 surfaces. Accounts client disabled offline. | connector | `support/http_test.go::*` |
| AC-USERS-03 | Fixture determinism, unknown user flat, collateral mutation, freeze upsert then delete. | model | `support/fixture_test.go::*` |
| AC-USERS-04 | US-01 … US-08. | model + golden | new |

## Chrome and keys

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-KEY-01 | Lookup table cases (screen verb, alternate key, global fallthrough, classes not table verbs); no default conflicts. | model | `ui/keymap_test.go::TestKeymapDefaultLookup`, `::TestKeymapNoDefaultConflicts` |
| AC-KEY-02 | Rebind drives dispatch and the hint; unknown or colliding rebinds rejected loudly, defaults kept. | model | `ui/keymap_test.go::TestKeymapRebindsVerb`, `::TestKeymapRejectsUnknownAndConflicts`, `::TestKeymapOverrideDrivesDispatch`, `::TestKeymapBrokenOverridesFallBackLoudly` |
| AC-KEY-03 | Symbol codes unique, explicit codes kept, no code uses a reserved verb key; and (new) no code collides with a NEWS verb (KM-11). | model | `ui/watchlist_test.go::TestAssignCodesUniqueAndReserved` + new |
| AC-KEY-04 | Screen cycle and help per screen (S16, S25). | model | `ui/watchlist_test.go::TestScreenCycle`, `ui/stream_test.go::TestStreamHelpOverlay`, `ui/orders_test.go::TestHelpOverlayToggle` |
| AC-KEY-05 | Keycast flow S24. | model | `ui/keycast_test.go::*` |
| AC-KEY-06 | The default view choice: with streaming off the order-entry view renders; with it on the BOOK chrome. (Glass: the screen set of OD-03.) | model | `ui/stream_test.go::TestDefaultViewUnchanged` |
| AC-KEY-07 | Every key reaches the app: arrows, Tab, Shift+Tab, F2, F3, F9, Esc, shifted digits. | e2e | [founder F-KEY-08] |

## Connections and folds

| ID | Criterion | Kind | From |
|---|---|---|---|
| AC-CONN-01 | Flows C1–C7 pass against in-process servers. | connector | `conn/live_test.go::*` |
| AC-CONN-02 | Venue routing, picker, read-only block, breadth default (S13–S15). | model | `ui/venue_test.go::*` |
| AC-CONN-03 | HL and Phoenix decoders: meta, fixed-point parse, book and trade frames, malformed side skipped, other channels ignored, coin filter, sector table. | connector | `conn/hyperliquid_test.go::*`, `conn/phoenix_test.go::*` |
| AC-CONN-04 | `/v1/symbols`: found, unknown id, server error, dead endpoint, ws→http mapping. | connector | `conn/symbols_test.go::*` |
| AC-CONN-05 | JWT shape, signature, numeric `exp`. | model | `conn/jwt_test.go::*` |
| AC-CONN-06 | Mock records; a down mock errors and records nothing; the demo script folds to LONG 14 @ 9998. | model | `conn/mock_test.go::*` |
| AC-CONN-07 | News decode, severity grades, window and order, ring cap, off stays off, the only dial site is Start. | model | `news/treeofalpha_test.go::*` |
| AC-CONN-08 | Market-data decode golden bytes; folder pairing; position checked arithmetic; latency window percentiles; seq gaps. | model | `rsx-core/wire/*_test.go`, `rsx-core/book/*_test.go` (survive) |
| AC-CONN-09 | GW-20 self-trade folds both legs; GW-21 unpaired fill never enters the position; GW-22 reason names; GW-24 double terminal idempotent; GW-25 accept upserts by oid. | model | new ([BUGS TERM-SELF-TRADE-FILL-FOLD], [BUGS TERM-FAILED-REASON-NOT-DECODED]) |
| AC-CONN-10 | MD-10 a seq gap re-subscribes; MD-11 stale after 2 s; MD-12 a crossed book is flagged. | connector | new |
| AC-CONN-11 | Live opt-in smokes: Hyperliquid standalone, the assistant chat turn. | e2e | `hl_smoke_test.go::TestHLStandaloneSmoke`, `assistant/smoke_test.go::TestLLMSmoke` |

## End to end (new)

| ID | Criterion | Kind |
|---|---|---|
| AC-E2E-01 | On `make demo`: place, see the accept, cancel, see the done; the round-trip p50 and p99 recorded. | e2e |
| AC-E2E-02 | A fill on the demo cluster builds the position shown, and it matches the gateway's account read when one exists. | e2e |
| AC-E2E-03 | Kill and restart the gateway: the order link reconnects with a fresh token, open orders restore from REST. | e2e |
| AC-E2E-04 | Kill marketdata: the book flags degraded within 2 s, recovers with a fresh snapshot. | e2e |
