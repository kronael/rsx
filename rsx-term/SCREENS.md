# SCREENS — rsx-term element inventory

Every screen the terminal renders, as an **element inventory**: each on-screen
element with its **position**, **what it encodes**, and its **states**. This is
the surface a GPU/bitmap successor must reproduce element-for-element. The
visual language (colours, glyphs, ramps, compressions) is `VISUALS.md`; the
render fns are cited `file:fn`; the end-to-end journeys are `FLOWS.md`.

There are **two render trees**, one model:

- **The classic DOM view** — the default, a three-column ratatui-style trading
  panel (`ui/view.go` `viewRoot`). Byte-locked by `TestDomViewGolden` /
  `TestBookViewGolden` / `TestDefaultViewUnchanged`.
- **The streaming "text Bookmap"** (`RSX_TERM_STREAM=1`, `ui/stream.go`
  `viewStream`) — **three screens** one keystroke apart (`tab` forward,
  `shift+tab` back): **BOOK → NEWS → LLM** (`screen.next`, `screenCount = 3`).

Both share the palette and the keycast chip. Symbol below: PENGU-PERP.

---

# The classic DOM view (default)

`viewRoot` stacks: status bar · [armed banner] · three columns · speed strip
(2 lines) · status line · help line (`lipgloss.JoinVertical`). Columns are
fixed content widths (`ui/view.go`): `bookWidth` 44, `orderWidth` 36,
`rightWidth` 34, `traceWidth` 46 (the F3 HUD). Below `bookWidth+orderWidth+
rightWidth` the layout stacks vertically (`narrow`).

**Status bar** (`viewStatusBar`) — top line.
- badge `RSX PENGU-PERP` — reverse-video, heading bg — the symbol.
- link dot `● live` / `● offline` — green / amber — order-gateway link.
- `open N  fills N` — muted — working-order and session-fill counts.
- `last <px>` — muted — last print, `—` before any.
- `~mark <px> (mid)` — `StyleDerived` — the client mid-mark, marked estimate.
- `index —  funding —` — muted — no source yet, honest dashes.

**Armed banner** (`viewArmedBanner`) — second line, only when `armed`.
- `⚠ ARMED — confirm OFF, orders fire on one enter · F2 to re-arm safety` —
  `StyleArmed` (bright on ask-red, bold), full-width, persistent. Absent when
  the two-enter confirm is on (the safe default).

**Book column** (`viewBook` / `ladderRow`) — a **static** price ladder.
- Position: left column. A fixed price axis centred on `resolvedCenter`
  (recentres only on drift — `ladderCenter`); row count from terminal height
  (`ladderRows`, 1–20).
- Per row `mark bidBar bidQty <price> askQty askBar`: `▊` depth bars scaled to
  the deepest visible level (bid grows left, ask right, converging on the
  spread); bid qty green left of the price, ask qty red right; price coloured by
  the resting side, muted in the empty spread band.
- Marks: own-order rows `▸` (accent), last-print row `‹` (bright).
- Bottom `imbalanceBar`: a `█` green|red split of visible depth + `NN% bid`.
- States: empty book or marketdata down → a single amber row
  `no live book — market-data stream down` (`degradedBookMsg`).

**Order column** (`viewOrder`) — the order form.
- Side buttons `  BUY  ` / `  SELL  ` — the active side `Reverse`d.
- `price:` / `qty:` fields — focused one bold+bright with a trailing `_`
  (`fieldLine`), other muted; you type the human decimal read off the ladder.
- `time-in-force: GTC/IOC/FOK`, `reduce-only: on/off   post-only: on/off`.
- Tail: `enter → preview, enter again to send` (muted) — OR the confirm preview
  (`viewConfirm`) when `pendingConfirm`: a violet `── confirm ──` divider, side
  in its hue, `notional`, TIF, `ro/po`, `liq n/a`, then bright
  `enter again to SEND · esc`.

**Right column** — positions, then working orders (only when non-empty), then
trades; OR the trace HUD when `showTrace` (it swaps the whole column).
- **Positions** (`viewPositions`) — panel `positions (mark=mid)`. Flat →
  `no position — fills build it`. Open → `LONG`(green)/`SHORT`(red) `+net @
  entry`, then `~uPnL <±notional>` (green/red, `StyleDerived` header) or amber
  `~uPnL — (needs live book)` with no mid, then the **risk row**
  (`viewRiskRow`): `liq —   ROE —   mgn ░░░░░░░░` + `(needs risk engine)`, all
  `StyleDerived` — the risk surface's fixed home, never a fabricated number.
- **Orders** (`viewOpenOrders`) — `side px qty`, `▸` (accent) marks the one `c`
  cancels (`↑↓` move it). Hidden when flat.
- **Trades** (`viewTrades`) — public tape, newest first, `B`/`S` glyph + price/
  qty coloured by taker side; `no trades yet` when empty.
- **Trace HUD** (`viewTrace`) — `RingPanelStyle`, `traceWidth` 46: LINKS
  (gw/md up/down + endpoints), LATENCY round-trip (net/internal/engine legs,
  p50/p99/best, sample count, sparkline), LATENCY marketdata (age/p50/since),
  FLOW (open/fills/spread/depth/last), + a pending-leg legend when legs unstamped.

**Speed strip** (`viewSpeed`) — two lines under the columns.
- `⚡ RTT <total>` — heading, amber past 50 µs, red past 100 µs — bold.
- `= net + internal + engine` legs (`·· pending` `StyleDerived` for unstamped),
  a `hopBar` (proportional `█`, per-leg hue, 18 cells).
- Line 2: `p50 · p99 · best` (muted) + sparkline (accent) + `· md <age>` /
  amber `· md stale`. Dim `⚡ latency: waiting for first round-trip…` before the
  first sample.

**Status line** (`viewStatusLine`) — one bright line, the last event / feedback.

**Help line** (`viewHelp`) — a static muted keybinding legend (`helpText`).

**Help overlay** (`viewHelpOverlay`, `?`) — a `RingPanelStyle` modal grouped by
function; destructive keys (`c`/`X`/`x`/`R`/`F2`/`q`) in ask-red. Any key closes.

---

# Streaming — BOOK (the depth heatmap)

`viewBookScreen` (`ui/stream.go`). A fixed grid repainted in place — history
lives in the heatmap ring, not scrollback. Top→bottom:

**Header** (`streamHeader`) — line 1.
- `RSX <name>` badge — reverse-video heading bg.
- `● live` / `● offline` — order-link dot (green / amber).
- `◀ bids  mid <px>  asks ▶` — the price-axis legend; `mid` = `—` until anchored.

**Mode line** (`modeLine`) — line 2, the "what will my next key do" strip.
- Screen tag `BOOK` — reverse-video accent block.
- venue (heading) · `RO` `PO` toggles (`StyleArmed` red ON / muted off) ·
  `size <qty> [n]` (armed preset 1–5) · `list <name>` (active watchlist).
- States: appends the `x` switcher buffer + candidate `code name · …` list while
  `switching`; the `F9` venue picker `venue? code Name · …` while `venuePicking`.

**Body** — the heatmap grid, top = oldest. Each row (`renderRow`) is
`rail + heatmap cells + gutter`, with the tape rail appended:
- **News rail** (col 0, `railChar`) — `newsRamp` severity marker (`·►►‼`) when an
  enabled source has a headline in the row's window, else `│`; the microscope's
  selected row shows `▸` (accent) instead.
- **Heatmap cells** (`cellStr`) — one glyph each: background `rampColor(sizeTier)`
  = resting size (ONE ramp, both sides), glyph `countRamp` = order count, `▚` =
  ≥30 s persistence, `◇` = your resting order, trade prints override with the
  side-shaped magnitude ramps (`▵▴△▲` buy / `▿▾▽▼` sell) in the aggressor hue
  over a contrast-capped background. Book side is **position** (left/right of
  the mid gap), never colour; trade side is **shape**, hue redundant.
- **Far rows** (top block) — aggregate time-weighted windows, dimmed; **live
  rows** — exact ~100 ms bins; blank rows pad between so the live block stays
  bottom-anchored and the grid is fixed.
- **NOW row** (bottom, `isNow`) — the current book every frame as `microRamp`
  micro-bars (linear, exact miniature histogram) carrying your `◇` orders.
- **Gutter** (col right, width 6, `gutterLabel`) — `−10s`…`−1h` horizon per far
  row, bright `now` on the NOW row, blank on live rows.
- **Tape rail** (right, `tapeRail`) — `┆` + the side-shaped magnitude glyph
  (`▵▴△▲`/`▿▾▽▼`) + exact price, one print per row, newest top, aggressor-hued.

**Ruler** (`rulerLine`) — the interaction axis under the NOW row: `│` + a `─`
baseline, the two touch columns `┼`, own orders `▲`/`▼` (accent, side by shape),
the price cursor `┃` (bright). `h`/`l` move the cursor, `f` fires at it.

**Footer** (`streamFooter`) — five pinned lines:
- `ask <px×qty>  <px×qty> …` (`touchLadderLine`, ask hue) — 3 exact ask levels,
  `—` when empty.
- `bid <px×qty> …` (live hue) — 3 exact bid levels.
- **Position line** (`streamPosLine`) — `pos flat — fills build it`, or `pos
  LONG/SHORT ±net @ entry   ~uPnL ±<notional>` (green/red), or amber `~uPnL —
  (needs live book)` with no mid.
- **Latency line** (`streamLatLine`) — `⚡ RTT <total>   p50 · p99 · best`, or
  `⚡ latency: waiting for first round-trip…` before the first RTT.
- **Hint line** (`hintLine`) — context-sensitive keys, generated from the keymap
  (`x symbol  n news  b/s side  h/l cursor  j/k touch  f place  d cancel …`).
  **Microscope status** (`microscopeLine`) replaces it while a row cursor is
  active: `microscope <honest window label>  · ↑↓ move · enter freeze →
  assistant · esc off`. Far row → `~10s window (aggregate, not a restored
  book)`; live row → `exact ~100ms bin`.

**Keycast chip** (`keycastChip`, `overlayBottomRight`) — bottom-right, only with
`RSX_TERM_KEYCAST=1`. `⌨ <key>  <label>` — key token as an accent block + the
keymap label; fades to muted over the last ~600 ms of its ~2 s life; in a
capture mode shows the bare key, no verb. Byte-identical (absent) when off.

**States**: `streaming heatmap — sizing…` before the first `WindowSizeMsg`;
`● offline` link dot; `ask —` empty-side ladder; `pos flat` position line.

**Help overlay** (`viewStreamHelp`, `?`) — `RingPanelStyle` modal, generated per
screen from the keymap (`helpLines`); the BOOK header notes orders fire on one
key. Any key closes.

---

# Streaming — NEWS (sector map + co-movement + feed)

`viewNews` (`ui/news_view.go`). Header + mode line, then:

**Sector map** (`sectorMapLines` / `tile`) — one line per sector, alphabetical.
- Position: below the mode line, capped to half the body height.
- Each line: a muted `sector` label + tiles. Each **tile** = `SYM   +N.NN%`,
  background a **diverging** blend (up = bid hue, down = ask hue) graded by
  `|move bp|` on `moveTiers` 10 / 50 / 200 / 800; foreground bright.
- States: no mid yet → a muted `—` tile (never a fabricated 0.00%); more symbols
  than fit → trailing `…`. Colourblind theme swaps the diverging pair to
  blue↔orange.

**Co-move row** (`coMoveLines` / `coMoveGlyph`) — one line.
- Position: under the sector map.
- `co-move vs BTC ~6s  ` prefix + per symbol a together-ness glyph + name,
  **most-coupled first**: `≡` strong-with (live bold) · `=` with · `·`
  independent (muted) · `≠` inverse (ask). Contemporaneous sign-agreement of
  ~6 s of 100 ms mid returns — a breadth read, NOT lead-lag.
- States: `gathering…` until `coMoveMinBins` (5) shared bins accumulate; `…` on
  overflow.

**Feed header** (`feedHeader`) — the divider over the feed.
- ` ── news feed ── / search · j/k select · enter → assistant ──` normally; the
  live `search: <query>_` buffer while searching; `── feed off — set
  RSX_TERM_NEWS=1 ──` when the source is disabled.

**Feed** (`feedLines`) — headlines, newest first, filling the remaining body.
- Per row: `▸` (accent) selection cursor or blank + muted `HH:MM:SS` + `newsRamp`
  severity marker + text `[+ SYMBOL tags]`, truncated with `…` to width.
- States: `no headlines yet — the feed fills as news lands` when enabled+empty.

**Status line** + **hint line** (`/ search · j/k select · enter → assistant ·
esc back`) close the grid. A symbol's **letter** dives into its BOOK.

---

# Streaming — LLM (the assistant pane)

`viewLLM` (`ui/news_view.go`). Header + mode line, then:

**Head** — `ASSISTANT` (heading, bold) + a muted mode note: `(no model wired —
the context below is exactly what one will receive)` offline, or `(live —
arizuko · reply streams below)` when `RSX_TERM_ASSIST` is set.

**Context block** (`assistContextLines`) — the frozen handoff (REAL either way):
- `── context handed off ──` divider.
- `market <venue> · <symbol>  at HH:MM:SS`.
- **Origin** (`assistOriginLines`) — a news headline (`headline <marker> <text>`
  + `HH:MM:SS · source · severity N`) OR a book-freeze note (`origin <label> ·
  <note>`, no fabricated marker).
- `book  mid <px>  (N bid / N ask levels frozen)`.
- `asks <px×qty …>` (ask hue) / `bids <px×qty …>` (live hue) — up to 3 levels.
- State: no handoff yet → `no context yet — select a headline (news) or freeze a
  row (book microscope) and press enter`.

**Reply pane** (`assistReplyLines`) — below the context.
- OFFLINE: `── assistant reply ──` + `~ placeholder — wiring an LLM is a
  follow-up; nothing here is generated` (`StyleDerived`) — byte-identical to
  pre-wiring, so the golden never drifts.
- LIVE: the streamed transcript (`assistTranscriptLines` / `wrapLogLine`,
  word-wrapped, role tags `you`/`asst`/`err` — err in ask-red, trimmed to the
  rows that fit) + the input caret `> <input>_`.

**Status line** — `⏳ waiting for the assistant… · <status>` when a turn is in
flight, else the last status. **Hint line** — `esc → news`.

---

# Cross-screen: venues, degradation, offline

**Venues** (`RSX_TERM_VENUE`) — `rsx` (default, no external dials) · `hyperliquid`
(standalone read-only over HL market data — all screens render; orders honestly
BLOCKED until HL signing lands) · `both` (RSX primary + HL breadth; NEWS defaults
to the HL universe, `F9` switches the BOOK's venue). Read-only venues render
fully but block orders with the reason.

**Degradation** (`RSX_TERM_COLOR`, `detectMode`) — `true` full ramp + all
channels; `shade` (16-colour) glyph density for size + aggressor hues, no ramp
background; `plain` glyphs only, zero escapes. Orthogonal: `RSX_TUI_THEME=
colorblind` swaps only bid/ask to blue/orange.

**Offline demo** — `RSX_GW_URL=mock go run .` runs the whole terminal against a
scripted feed (`conn/mock.go`) with no network — the reference for this
catalogue. On the mock all three latency legs are real; live, only `net` is
client-measured until the gateway stamps internal/engine (`·· pending`).
