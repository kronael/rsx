---
status: draft
---

# 07 — Screen: assistant (LLM)

The assistant screen is a chat with an LLM agent that receives exactly what
the trader was looking at — a news headline with its market's frozen book, or
a frozen heatmap row — and replies in a streamed transcript. It is a chatting
interface, never a trading automaton [founder F-AST-02, F-AST-05]. The wire
protocols (arizuko chat SSE, the runner and MCP target) are in
[14-contract-support-assistant.md](14-contract-support-assistant.md); keys in
[10-keymap.md](10-keymap.md); acceptance in [17-acceptance.md](17-acceptance.md) § LLM.

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Layout

Offline, after a news handoff (rendered from the pinned code; blank rows
collapsed to `…`) [code rsx-term/ui/screendump_test.go]:

```
 RSX  PENGU-PERP   ● offline  ◀ bids  mid 0.010000  asks ▶
 LLM  rsx  RO off  PO off  size 1.0000 [1]  list rsx

  ASSISTANT  (no model wired — the context below is exactly what one will receive)

  ── context handed off ─────────────────────────
  market   rsx · SOL-PERP  at 07:50:51
  headline ‼ SOL ETF speculation lifts the majors
           05:20:00 · Twitter · severity 3
  book     mid 149.3700  (1 bid / 1 ask levels frozen)
  asks 149.3701×0.000040
  bids 149.3699×0.000040

  ── assistant reply ────────────────────────────
  ~ placeholder — wiring an LLM is a follow-up; nothing here is generated
…
 context → assistant: SOL-PERP
 esc → news  q quit  tab view  r RO  ? help
```

With no handoff yet the body is one line:
`  no context yet — select a headline (news) or freeze a row (book microscope) and press enter`
[code rsx-term/ui/news_view.go:525-527].

## Elements

| Element | Content | Source |
|---|---|---|
| head | `ASSISTANT` (Heading, bold) + Muted `(no model wired — the context below is exactly what one will receive)` offline, or `(live — arizuko · reply streams below)` live | [code rsx-term/ui/news_view.go:519-523] |
| context block | `── context handed off ──`; `market <venue> · <symbol>  at HH:MM:SS`; origin lines; `book  mid <px>  (N bid / M ask levels frozen)` (`mid —` when unknown); `asks …` / `bids …` up to 3 levels each (`asks —` when empty) | [code rsx-term/ui/news_view.go:544-564, 687-701] |
| origin — news | `headline <marker> <text>` and `HH:MM:SS · <source> · severity <n>` | [code rsx-term/ui/news_view.go:679-685] |
| origin — book freeze | `origin book freeze · <window note>` (e.g. `~3s window (aggregate, not a restored book)` or `exact ~100ms bin`); never a headline or severity | [code rsx-term/ui/news_view.go:668-678], [code rsx-term/ui/microscope_test.go TestBookFreezeAssistantRendersOriginNote] |
| reply — offline | `── assistant reply ──` + derived `~ placeholder — wiring an LLM is a follow-up; nothing here is generated` (byte-locked) | [code rsx-term/ui/news_view.go:566-579], [code rsx-term/ui/news_view_test.go TestAssistReplyLinesOfflineExact] |
| reply — live | `── assistant reply ──`; the transcript word-wrapped to `width−4` (min 24), role tags `you ` (TextBright) / `asst` (Text) / `err ` (Ask), continuation lines indented, trimmed to the newest rows that fit; then the input `  > <draft>_` | [code rsx-term/ui/news_view.go:580-666] |
| status | `⏳ waiting for the assistant… · <status>` while a turn is in flight; else the last status (mid-turn progress text arrives here) | [code rsx-term/ui/news_view.go:535-539] |

## The handoff (UI-agnostic contract)

```go
// rsx-term/news/context.go:36-54 (exact, comments trimmed)
type AssistantContext struct {
	Origin Origin
	Venue  string
	Symbol string
	TsNs int64
	Headline *Marker
	Note string
	Bids []wire.Level
	Asks []wire.Level
	MidPx int64
}
```

`Origin` is `OriginNews` (a selected headline, `Headline` set) or
`OriginBookFreeze` (a microscope row, `Headline` nil, `Note` the honest window
label). Levels are deep-copied so the context stays frozen while the live book
folds on [code rsx-term/news/context.go:5-95]. A book freeze carries the
row's price-space levels split by side and the heatmap's anchored mid
[code rsx-term/ui/news_view.go:445-483].

The prompt sent as the opening turn is rendered by a pure function of the
context and the trader's client-folded state:

```go
// rsx-term/assistant/prompt.go:58-83 (exact)
func Render(ctx news.AssistantContext, snap Snapshot) string {
	var b strings.Builder
	b.WriteString("[RSX CONTEXT]\n")
	b.WriteString("origin: " + ctx.Origin.Label() + "\n")
	ts := time.Unix(0, ctx.TsNs).UTC().Format("2006-01-02 15:04:05 UTC")
	b.WriteString("market: " + ctx.Venue + " · " + ctx.Symbol + "  at " + ts + "\n")
	if ctx.Origin == news.OriginNews && ctx.Headline != nil {
		h := *ctx.Headline
		b.WriteString(fmt.Sprintf("headline: %s  (%s · severity %d)\n", h.Text, h.Source, h.Tier))
	} else if ctx.Note != "" {
		b.WriteString("note: " + ctx.Note + "\n")
	}
	mid := "—"
	if ctx.MidPx > 0 {
		mid = decimal(ctx.MidPx, snap.PriceDec)
	}
	b.WriteString("mid: " + mid + "\n")
	b.WriteString("asks: " + levelText(ctx.Asks, snap.PriceDec, snap.QtyDec) + "\n")
	b.WriteString("bids: " + levelText(ctx.Bids, snap.PriceDec, snap.QtyDec) + "\n")

	b.WriteString("\n[TRADER STATE]\n")
	b.WriteString("position: " + positionText(snap) + "\n")
	b.WriteString("open orders: " + ordersText(snap) + "\n")
	b.WriteString(fmt.Sprintf("fills this session: %d\n", snap.Fills))
	return b.String()
}
```

Trader state: `flat`, or `+net  entry <px>  uPnL ±<money>` (entry and uPnL
dashed when unavailable; uPnL marked at the frozen mid); open orders
`BUY <px>×<qty>, …` or `none`; the session fill count
[code rsx-term/assistant/prompt.go:85-145], [code rsx-term/ui/news_view.go:773-803].

## Threads and turns

| Rule | Detail | Source |
|---|---|---|
| one thread per handoff | each handoff mints `t-<unixms>-<venue>-<symbol>`, clears the transcript, and posts the rendered prompt as the first turn; typed follow-ups reuse the topic | [code rsx-term/ui/news_view.go:758-771, 805-810] |
| one turn in flight per thread | `enter` while busy keeps the draft and says `assistant is busy — wait for the reply` (a second POST on the same topic would cross-subscribe the reply stream) | [code rsx-term/ui/news_view.go:728-745] |
| stale thread | a reply, status or failure for another topic is dropped | [code rsx-term/ui/update.go:121-134], [code rsx-term/ui/news_view_test.go TestLLMIgnoresStaleThreadReply] |
| failure | an `err` row `assistant unreachable — <reason>` and the busy marker clears; never a fabricated reply | [code rsx-term/ui/update.go:130-134] |
| offline | no client: the pane is the placeholder, zero dials, busy never set | [code rsx-term/ui/news_view_test.go TestLLMOfflinePaneUnchanged] |

## Requirements beyond the terminal

The terminal ships only the handoff-driven pane above. The founder's design
(spec `specs/2/60-terminal-assistant.md`, draft; founder-confirmed 2026-07-11)
adds the following; none of it exists in code at the pin.

| ID | Requirement | Source |
|---|---|---|
| AS-01 | A chat key on every screen opens the chat with that screen's vantage; the chat is reachable without a handoff. | [founder F-AST-03 "every page should have a chat input view"], KM-20 |
| AS-02 | Every message carries a vantage block: the screen, its focus (symbol, cursor, frozen row, selected headline, selected user), and the full folded state, rich and full for now (bounded: top ~5 levels per side, about 30 lines), with the same honesty rules. | [founder F-AST-03 "just make it for now ruch and full"], [doc specs/2/60-terminal-assistant.md "Channel 1"] |
| AS-03 | The agent can read every screen's state live through MCP tools (`rsx_get_screen`, `rsx_get_orderbook`, `rsx_get_trades`, `rsx_get_positions`, `rsx_get_orders`, `rsx_get_fills`, `rsx_get_news`, `rsx_get_accounting`) served from an immutable snapshot of the UI's own folds. | [founder F-AST-03 "everything sent in the mcp"], [doc specs/2/60-terminal-assistant.md "Read surface"] |
| AS-04 | The agent can navigate and focus (`rsx_goto_screen`, `rsx_focus_symbol`, `rsx_set_cursor`, `rsx_freeze_row`) through typed semantic messages folded by the same guarded state machine as keys — never synthesized keystrokes. | [doc specs/2/60-terminal-assistant.md "Concurrency bridge"] |
| AS-05 | Order entry never executes agent-side. A proposed order (`rsx_propose_order`, if the founder admits it — OD-11) only pre-fills a ticket the trader key-confirms; the submit transition has no injectable equivalent; the fat-finger caps bind it. | [doc specs/2/60-terminal-assistant.md "Control surface"], [founder F-AST-05 "not a trading automaton"] |
| AS-06 | The backend is selectable: the existing HTTP chat client, or the local runner (arizuko's Claude Code container, unchanged); unset = the offline placeholder. | [founder F-AST-04], [doc specs/2/60-terminal-assistant.md "Backend seam"] |
| AS-07 | The agent's network egress is decided before it ever sees a real account (OD-12): headlines are attacker-controlled text in its context. | [doc specs/2/60-terminal-assistant.md "Open questions" 0] |
| AS-08 | Waiting shows progress (the runner's mid-turn status) in the status line; a turn has an inactivity cutoff, not a total deadline. | [code rsx-term/assistant/client.go:14-19] |
