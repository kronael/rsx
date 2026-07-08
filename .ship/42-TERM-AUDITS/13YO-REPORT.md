# rsx-term usability audit — 13-year-old / brand-new-trader lens

Scope: `rsx-term/` (Go terminal), read-only audit against `specs/2/55-terminal.md`
new-trader requirements. Founder framing: keyboard-driven is fine — this audit
dings **confusion, hidden danger, and missing survival info**, not learning
curve.

Verified against the actual Go source (`ui/`, `book/`, `conn/`, `wire/`,
`main.go`), not the spec's own scorecard — the spec table (55-terminal.md
:171-205) is **stale**: it claims confirm-preview is still "near-term
missing," but rsx-term already ships it.

---

## Ranked findings (worst-for-a-novice first)

### 1. [DANGER, real gap in copy only] No liquidation price — and the placeholder reads like a debug string, not a warning
- MUST-HAVE #1: **MISSING**, honestly labeled.
- `ui/view.go:183`: `line3 := StyleMuted.Render("liq — (needs server)")` inside
  the confirm-preview box.
- Root cause is a genuine server gap, not an rsx-term bug: `book/position.go`
  only tracks `Net`/`Cost` from own fills (lines 11-14) — there's no
  margin/leverage/liq concept anywhere client-side to derive it. Matches
  `specs/2/55-terminal.md:181` ("needs risk margin data; no source yet").
  **Legitimately Post-MVP — label honestly, no client fix possible.**
- **Real, cheap rsx-term fix that IS available today**: the copy itself. A
  13-year-old doesn't parse "liq — (needs server)" as "you can be wiped out
  and we can't tell you where." Rewrite to something that names the risk in
  plain English even while the number is absent, e.g. `liq: unknown — this
  account CAN be liquidated`.

### 2. [DANGER, real gap] uPnL silently shows "—" with zero explanation when the book can't produce a mid
- `ui/view.go:215-225`: if `m.book.Mid()` returns `false` (one-sided or empty
  book), `upnl` stays `"—"` with no caption telling the user why.
- Contrast: the book-empty state elsewhere gets an explicit amber message
  (`degradedBookMsg`, const at `ui/view.go:24`: `"no live book — market-data
  stream down"`), but the position panel's own dash has no equivalent. A
  beginner cannot tell "flat," "broken," and "can't compute right now" apart.
- **Real code gap, cheap fix**: add a muted caption line next to the dash
  when `Upnl()` fails. Not blocked on server data — `Mid()` is already
  entirely client-derived.

### 3. [MISLEADING — client-derived value that reads as authoritative] "mark" and uPnL are computed from the local order-book mid, not any exchange number, and the labeling doesn't carry enough visual weight
- `book/book.go:124-138` `Mid()` — averages best bid/ask from the locally
  folded ladder (BBO fallback). Pure client computation; **no server "mark"
  field exists anywhere in `wire/`** (`wire/md.go:212-260` — only
  Bbo/Snapshot/Delta/MdTrade/Heartbeat, no Mark/Index message type).
- `ui/view.go:70-74`: status bar renders `mark %s (mid)` crammed into one
  dense line next to `last`, `index —`, `funding —`, all in identical muted
  styling.
- `ui/view.go:198-234` `viewPositions()`: panel title `" positions (mark=mid)
  "` and uPnL via `m.position.Upnl(mid)` at `ui/view.go:217-224`; label
  exists but sits in `StyleMuted` (`styles.go:54`) panel-title text — easy to
  skip.
- `book/position.go:67-73` `Upnl()`: `Net*mark - Cost`, correctly documented
  in a code comment as client-derived, but that honesty never reaches the
  rendered UI with matching visual weight.
- **This is the single most dangerous mislabeling risk in the terminal.** A
  beginner can watch a green uPnL number next to a "mark" price and believe
  both are exchange-confirmed facts, when they're a local fold of whatever
  the client's own book currently shows — which can be stale, gapped, or
  momentarily one-sided.
- Real exchange mark price is legitimately Post-MVP (needs the `15-mark.md`
  feed; confirmed absent from wire types). **The visual-weight problem is a
  real, cheap rsx-term fix** — dim/italicize the derived numbers, or prefix
  with `~` (e.g. `mark~10000`), so "estimate" reads differently from "fact"
  at a glance, not just in a text suffix a rushed reader skips.

### 4. [DANGER, legitimately Post-MVP] No available margin / free balance / notional-vs-balance check anywhere
- MUST-HAVE #4: **MISSING**.
- `ui/update.go:228-245` `buildOrder()` only validates px/qty parse as
  positive int64 — no balance, no margin, no notional cap.
- `book/` has no `Balance`/`Margin`/`Collateral` type at all.
- The confirm preview does show notional (`ui/view.go:182`: `notional %d`),
  which is good, but there's nothing to compare it against — the terminal
  literally cannot compute "that's 340% of your balance" because balance
  isn't tracked client-side and no account-query wire message (`A`) exists.
- **Legitimately Post-MVP / needs server** — matches spec honesty table
  (55-terminal.md:336). Not fakeable client-side.

### 5. [CONFUSION, legitimately Post-MVP but silent] No leverage concept anywhere in the order path
- MUST-HAVE #6: **MISSING**.
- `wire/types.go:61-68` `OrderReq` has no leverage field; the order form
  (`ui/view.go:137-158`) shows side/price/qty/tif/ro/po — no leverage
  selector or display next to qty.
- Matches spec's honesty table (55-terminal.md:339: leverage/margin-mode
  "needs server"). Arguably the most severe Post-MVP gap *because it's
  silent* — a beginner has zero signal that leverage even applies to their
  order. Combined with finding #1 (no liq price), a beginner literally
  cannot reason about downside risk from the terminal alone.
- No client fix possible without server data; **the honest move is to make
  the absence loud** (e.g., a visible "leverage: server does not report
  this yet" line) rather than simply omitting the field.

### 6. [CONFUSION, real gap] Confirm-before-submit exists and is good — but a beginner can blow past it with fast double-enter, and the hint text doesn't explain the two-stage behavior
- Positive: MUST-HAVE #5 is **fully implemented**, ahead of the spec's own
  claim that it's still "near-term" (55-terminal.md:185).
  - `ui/update.go:196-224` `handleEnter()`: first `enter` builds
    `pendingConfirm` (preview only); a **second** `enter` calls
    `m.cfg.Sub.Submit(o)`.
  - `ui/view.go:178-187` `viewConfirm()` renders side (colored + text label,
    not color-only), qty, price, notional, TIF, ro/po flags, and the liq
    placeholder, inside a distinct violet-ring border
    (`RingPanelStyle`, `styles.go:76-78`).
  - `esc` cancels and shows `"order not sent"` (`ui/update.go:121-126`).
  - Editing the form after a preview is built silently discards the pending
    confirm (`ui/update.go:164-166`) — correct, prevents sending a stale
    preview after a field change.
- **The real gap**: the persistent hint at the bottom of the order panel
  just says `"enter → confirm"` (`ui/view.go:152`) — singular, doesn't
  distinguish "first enter previews, second enter sends." There is no
  minimum delay or distinguishing keystroke between preview and send —
  two fast `enter` presses (muscle memory from other apps) submits with no
  friction beyond a re-render.
- **Real, cheap rsx-term fix**: require a different key to confirm (e.g.
  `y`) instead of reusing `enter`, or add a short debounce, and fix the hint
  text to say `enter → preview, enter again → send`.

### 7. [MOSTLY DONE, spec was stale] Side coloring + long/short labels
- MUST-HAVE #2: **present**, better than the spec's own "partial" claim
  (55-terminal.md:182).
  - Order form (`ui/view.go:138-145`): BUY is bid-green, SELL is ask-red;
    active side additionally shown with `.Reverse(true)` inverted block —
    color AND text, not color-only.
  - Positions panel (`ui/view.go:204-208`): explicit word `"LONG"`
    (green, `StyleLive`) or `"SHORT"` (red, `StyleAsk`) based on `net < 0`.
    Confirmed by golden test `TestViewLiveBookAndPosition`
    (`ui/view_test.go:76-78`).
- **One real remaining gap**: the trade tape is colored by taker side
  (`ui/view.go:252`, `sideColor(e.Side)`) with **no B/S text at all** —
  color-only. A colorblind beginner cannot read taker side from the tape.
  Minor, SHOULD-HAVE-adjacent, cheap fix (prefix each row with `B`/`S`).

### 8. [PARTIAL] uPnL in $ and ROE% — $ done and colored, ROE% missing
- MUST-HAVE #3: $ uPnL present and colored (`ui/view.go:217-224`, green/red
  via sign). ROE% requires knowing margin/leverage, which per findings #4/#5
  doesn't exist client-side. **Legitimately Post-MVP**, consistent root
  cause with #4/#5.

### 9. [PARTIAL] Mark vs last — both labeled by name, not by trust level
- MUST-HAVE #7: technically present — both `last` and `mark` have text
  labels (`ui/view.go:65-74`) — but as covered in finding #3, "last" is a
  genuinely server-sent trade print (`wire.MdTrade` via `m.tape.Last()`,
  `ui/view.go:65-68`) while "mark" is a local mid estimate, and nothing in
  the rendering differentiates *trustworthiness*, only *name*. Same fix as
  finding #3 (visual differentiation, not more text).

---

## Should-haves

| # | Item | Verdict | Note |
|---|---|---|---|
| 10 | Funding rate + countdown | **Missing, honestly labeled** | `ui/view.go:74`: status bar always renders `funding —`, no fabricated rate. No funding message type in `wire/`. Legitimately Post-MVP — correctly absent rather than faked. |
| 11 | Margin-ratio / distance-to-liq bar | **Missing** | Same root cause as #1/#4/#5 — no margin data client-side. Post-MVP. |
| 12 | Size as % of balance | **Missing** | Same root cause as #4 — no balance tracked client-side. Post-MVP. |
| 13 | Reduce-only default on close | **Missing — real gap** | There's no dedicated "close position" action at all; a user must manually flip side and set qty, and `reduceOnly` defaults false even then (`ui/model.go:66`, zero value). Nothing suggests enabling it when the order would reduce/flip the position. **Real, cheap rsx-term fix**: detect "this order reduces the existing position" and default `ro=true` / prompt. |
| 14 | Spread/BBO visible | **Done** | `ui/view.go:111`, spread row `"— %d —"` via `book/book.go:112-122` `Spread()` (best-ask − best-bid, 0 if either side missing, never fabricated from one side). Depth bars scaled and capped (`ui/view.go:20,127`). |

---

## First-10-seconds walkthrough (mock demo)

Reconstructed from `conn/mock.go:69-105` (scripted at 30ms/msg) plus
`ui/view.go` render logic and `ui/view_test.go` golden strings (a live TUI
render wasn't captured — non-TTY).

Sequence: `GwUp`/`MdUp` → link dots green → `Snapshot` (book renders
instantly, spread shows) → two `MdTrade`s (tape + last price populate) →
`Accepted` (order 7 accepted, open-orders count +1) → `Fill` (position flips
to LONG) → `Done` → two `Latency` samples (speed strip fills in).

- **(a) What market they're in**: YES, immediately. Bold violet badge
  `" RSX  PENGU-PERP "` top-left from frame 1 (`ui/view.go:52-56`).
- **(b) Up or down**: PARTIALLY, only after ~4 scripted events. Before the
  fill, the position panel honestly says `"no position — fills build it"`
  (`ui/view.go:201`). After the fill, LONG + colored uPnL appear — but per
  finding #3, that green number is client-mid-derived and isn't visually
  flagged as an estimate strongly enough; a first-timer will plausibly read
  it as exchange-confirmed.
- **(c) Place + confirm an order**: PARTIALLY discoverable. The bottom help
  legend (`ui/view.go:27`, `helpText`) lists `"enter submit"`, but per
  finding #6 the two-stage preview-then-send behavior isn't spelled out
  there or in the order panel's own hint (`"enter → confirm"`,
  `ui/view.go:152`). No onboarding/first-run explainer exists.

**Single most confusing/dangerous thing overall**: the combination of no
liquidation price + no leverage indicator anywhere (findings #1, #5), paired
with a client-computed "mark"/uPnL (finding #3) that visually presents with
the same confidence as real exchange data. A beginner can watch a friendly
green "+28" next to LONG, feel safe, and have no way to know from the screen
how much room exists before forced liquidation — because that number doesn't
exist client-side at all, and the one number that does exist (mid-derived
mark) isn't visually distinguished from authoritative data.

## Unlabeled / ambiguous items (minor, real, cheap fixes)

- `ro`/`po` in the order form (`ui/view.go:150`: `"ro   : %s   po: %s"`) are
  raw abbreviations, never expanded anywhere in the UI or help legend.
  Beginner has no way to learn "reduce-only" / "post-only" from the screen.
- `tif` shows `GTC`/`IOC`/`FOK` (`wire/types.go:36-45`) with zero
  explanation anywhere in the rendered UI.
- Status-bar `open %d  fills %d` (`ui/view.go:63`) — "open" means open
  *orders*, not open *position size*; easily misread as position size or
  open interest since the positions panel is elsewhere on screen.

---

## Scorecard (verified independently of the spec's own table)

| # | MUST-HAVE | Verdict | Fix class |
|---|---|---|---|
| 1 | Liq price prominent | Missing (honestly dashed) | Post-MVP (data) + real fix (copy) |
| 2 | Side color + long/short label | Present (tape is color-only, minor) | Done / minor real fix |
| 3 | uPnL $ + ROE%, live, colored | Partial ($ done; ROE% missing) | Post-MVP (ROE needs margin) |
| 4 | Available margin/balance | Missing | Post-MVP |
| 5 | Confirm-before-submit preview | Present, but fast-double-enter bypass risk | Real fix (friction + copy) |
| 6 | Leverage next to size | Missing, silent | Post-MVP (data) + real fix (make absence loud) |
| 7 | Mark vs last, both labeled | Partial (named, not trust-differentiated) | Real fix (styling) |

| # | SHOULD-HAVE | Verdict |
|---|---|---|
| 8 | Funding + countdown | Missing, honestly dashed (Post-MVP) |
| 9 | Margin-ratio/distance-to-liq bar | Missing (Post-MVP) |
| 10 | Size as % of balance | Missing (Post-MVP) |
| 11 | Reduce-only default on close | Missing (real gap, no close action exists) |
| 12 | Spread/BBO visible | Done |

## Real code gaps worth fixing now (not blocked on server data)

1. uPnL dash needs a reason caption when `Mid()` fails (finding #2).
2. Visually differentiate client-derived numbers (mark, uPnL) from
   server-confirmed ones (last) — dim/italic or a `~` prefix, not just a
   text suffix (findings #3, #9).
3. Confirm flow: distinguish preview-enter from send-enter with a different
   key or a debounce; fix the hint text (finding #6).
4. Reduce-only should default on when an order would reduce/flip the
   existing position (finding #13).
5. Expand `ro`/`po`/`tif` abbreviations somewhere reachable (help legend or
   inline).
6. Sharpen the "liq — (needs server)" placeholder copy into an actual
   plain-English risk warning even while the number is unavailable
   (finding #1).
7. Trade tape: add a B/S text prefix so side isn't color-only (finding #7).

## Legitimately Post-MVP (label honestly, no client-side fix possible)

Liquidation price, leverage, available margin/balance, ROE%,
margin-ratio/distance-to-liq bar, size-as-%-of-balance, funding rate +
countdown, true exchange mark price. All trace to the same root cause: no
margin/leverage/account-balance/mark-price data exists anywhere in
`wire/` today (confirmed by reading `wire/types.go` and `wire/md.go` in
full — no `A` account-query message, no Mark/Index/Funding message types).
The terminal is already doing the honest thing for all of these (dashing
them out, not fabricating), except for the "mark=mid"/"liq needs server"
cases where a real, cheap terminal-side change (heavier visual/copy
treatment) would materially reduce the risk of a beginner mistaking an
estimate for a fact.
