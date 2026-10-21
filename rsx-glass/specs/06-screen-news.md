---
status: draft
---

# 06 — Screen: NEWS

NEWS is the market-context screen: a sector map of the breadth venue's
symbols coloured by recent move, a cross-symbol co-movement line against a
BTC (or ETH) reference, and the searchable headline feed, from which one key
jumps into a symbol's BOOK and `enter` hands a headline plus that market's
frozen book to the assistant. It is a watch surface, not a trading grid
[founder F-SCR-05, F-SCR-06]. Chrome: [09-chrome.md](09-chrome.md); keys:
[10-keymap.md](10-keymap.md); handoff: [07-screen-assistant.md](07-screen-assistant.md);
feed contract: [13-contract-marketdata.md](13-contract-marketdata.md) § News;
acceptance: [17-acceptance.md](17-acceptance.md) § NEWS.

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Layout

Frame rendered from the pinned code by `TestDumpScreens` (majors on the
primary venue, three headlines; 30 blank rows collapsed to `…`)
[code rsx-term/ui/screendump_test.go]:

```
 RSX  PENGU-PERP   ● offline  ◀ bids  mid 0.010000  asks ▶
 NEWS  rsx  RO off  PO off  size 1.0000 [1]  list rsx
 majors   BTC    +0.56%  ETH    +0.42%  SOL    -0.42%  XRP    +0.23%
 meme     PENGU  +0.01%
 co-move vs BTC ~6s  ≡ ETH   ≡ XRP   ≠ SOL
 ── news feed ── / search · j/k select · enter → assistant ──
▸ 05:20:00 ‼ SOL ETF speculation lifts the majors [SOLUSDT]
  03:33:20 ► XRP appeal filed; volatility spikes [XRPUSDT]
  01:46:40 ► ETH staking inflows rise [ETHUSDT]
…
 connecting…
 / search  j/k select  enter → assistant  esc back  q quit  tab view  r RO  ? help
```

Stack: header · mode line · sector map (at most half the body) · co-move
line · feed header · feed rows (the rest) · padding · status line · hint line;
exactly the window height [code rsx-term/ui/news_view.go:39-69],
[code rsx-term/ui/stream_golden_test.go TestNewsViewFixedShape].

## Elements

| Element | Encodes | Data source | Source |
|---|---|---|---|
| mode-line venue | the breadth venue (the active watchlist's), not the BOOK venue | watchlist | [code rsx-term/ui/stream.go:588-591] |
| sector map | one line per sector (alphabetical; instruments with no sector → `perps`); muted label then tiles; `…` when more tiles than fit (`(width−9)/15` per line) | breadth venue instruments + each market's mid history | [code rsx-term/ui/news_view.go:71-110] |
| tile ` SYM   +N.NN% ` (15 wide) | name before the first `-`; move in bp shown as a signed percent; ground on a diverging blend graded by `|bp|` against 10 / 50 / 200 / 800 bp, up = bid hue, down = ask hue (blue/orange in the colourblind theme); TextBright ink; no mid yet → a Muted `—` tile | market mid vs its oldest retained mid sample | [code rsx-term/ui/news_view.go:112-160], [code rsx-term/ui/market.go:152-160] |
| co-move line | `co-move vs <REF> ~6s ` then, most-coupled first, a coupling mark + name per symbol; `gathering…` until enough shared bins; `…` past `(width−24)/8` entries | per-bin mid rings of each symbol and the reference | [code rsx-term/ui/news_view.go:162-286] |
| feed header | ` ── news feed ── / search · j/k select · enter → assistant ──`; while searching `── search: <query>_ ──`; with a kept filter `── search: <query> ──`; source off `── feed off — set RSX_TERM_NEWS=1 ──` | news source state | [code rsx-term/ui/news_view.go:288-303] |
| feed row | `▸ ` (Accent) on the selection else two spaces; Muted `HH:MM:SS `; severity marker; text + ` [TAG TAG]`; truncated with `…` to `width−16` | headline buffer, filtered | [code rsx-term/ui/news_view.go:305-335] |
| empty feed (source on) | `   no headlines yet — the feed fills as news lands` | | [code rsx-term/ui/news_view.go:331-333] |
| status / hint | last status; generated hint | | [code rsx-term/ui/news_view.go:66-67] |

Co-movement is the contemporaneous sign agreement of per-bin mid returns over
the overlap of two ~6 s rings (64 bins of 100 ms); flat bins say nothing; at
least 5 shared bins or no figure. It is a together-ness read, deliberately NOT
lead-lag [founder F-SCR-06], [code rsx-term/ui/news_view.go:162-202]:

```go
// rsx-term/ui/news_view.go:174-202 (exact)
func coMove(sym, ref []int64) (float64, bool) {
	n := len(sym)
	if len(ref) < n {
		n = len(ref)
	}
	if n < coMoveMinBins {
		return 0, false
	}
	sym = sym[len(sym)-n:]
	ref = ref[len(ref)-n:]
	agree, disagree := 0, 0
	for i := 1; i < n; i++ {
		ds := sign64(sym[i] - sym[i-1])
		dr := sign64(ref[i] - ref[i-1])
		if ds == 0 || dr == 0 {
			continue // a flat bin says nothing about direction
		}
		if ds == dr {
			agree++
		} else {
			disagree++
		}
	}
	total := agree + disagree
	if total == 0 {
		return 0, false
	}
	return float64(agree-disagree) / float64(total), true
}
```

```go
// rsx-term/ui/news_view.go:225-238 (exact)
func coMoveGlyph(co float64) string {
	switch {
	case co >= 0.6:
		return StyleLive.Bold(true).Render("≡")
	case co >= 0.25:
		return StyleLive.Render("=")
	case co <= -0.6:
		return StyleAsk.Bold(true).Render("≠")
	case co <= -0.25:
		return StyleAsk.Render("≠")
	default:
		return StyleMuted.Render("·")
	}
}
```

Reference: the first instrument whose base name starts with `BTC`, else `ETH`,
else the venue's first instrument [code rsx-term/ui/news_view.go:204-220].

## Behaviour

| Behaviour | Rule | Source |
|---|---|---|
| search | case-insensitive substring over text, source and tags; typing resets the selection; enter keeps the filter, esc clears it | [code rsx-term/ui/news_view.go:337-407] |
| letter jump | a symbol's code opens its BOOK on the breadth venue | [code rsx-term/ui/news_view.go:375-382] |
| handoff | `enter` packages the selected headline + the linked market's book (deep-copied) + its mid; the link is the first tag that equals an instrument name or starts with its base; an unlinked headline hands off the active BOOK market instead; empty feed → `no headline selected (feed empty)` | [code rsx-term/ui/news_view.go:418-443, 495-512] |
| default breadth venue | the widest configured venue (an extra venue such as Hyperliquid lists before the primary) | [code rsx-term/ui/model.go:441-451], [code rsx-term/ui/venue_test.go TestWatchlistDefaultsToBreadthVenue] |

## Requirements beyond the terminal

| ID | Requirement | Source |
|---|---|---|
| NW-01 | The tile's reference is named on screen. At the pin the move is measured against the oldest retained mid sample — about 6.4 s ago — while the code calls it "the session reference"; the founder asked for "where they move relative to a reference". Glass shows the reference window in the sector-map header (e.g. `move vs 6s`), and which reference to use (session open, 24 h, 6 s) is OD-09. | [code rsx-term/ui/market.go:55-56, 152-160], [founder F-SCR-05] |
| NW-02 | A negative move keeps its minus sign at every magnitude (`-0.04%`, never `0.04%`). | [code rsx-term/ui/news_view.go:122-133] (fixed in the terminal; keep it) |
| NW-03 | Watchlists are switchable: the model holds named lists (one per venue) but no key changes the active list at the pin (`listSel` never moves; the `[` / `]` mentioned in a comment are unbound). Glass binds list switching or removes the `list` field from the mode line (OD-10). | [code rsx-term/ui/market.go:122-129], [code rsx-term/ui/model.go:294-297] |
| NW-04 | Symbols whose codes are bound verbs on NEWS (`j`, `k`) are reachable (KM-11). | [10-keymap.md](10-keymap.md) |
| NW-05 | The severity marker states it is a heuristic in help or a legend (Tree of Alpha carries no severity). | [code rsx-term/news/treeofalpha.go:316-320], [doc specs/2/55-terminal.md honesty table "severity is a keyword heuristic"] |
| NW-06 | The co-move line names its window and reference (`co-move vs BTC ~6s`) and never computes lead-lag. | [doc rsx-term/CLAUDE.md "no lead-lag over jittery 100 ms mids"] |
| NW-07 | Headline time zone is stated (local `HH:MM:SS` at the pin; the assistant prompt uses UTC). | [code rsx-term/ui/news_view.go:319], [code rsx-term/assistant/prompt.go:62] |
| NW-08 | The chat key opens the assistant with the NEWS vantage (selected headline, breadth venue). | KM-20, [founder F-AST-03] |
