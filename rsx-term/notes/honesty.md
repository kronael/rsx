# Honesty over fabrication — never show a number you can't stand behind

A trading terminal shows numbers a human bets real money on, in seconds, under
pressure. Its cardinal sin is not being ugly or slow — it's showing a
*plausible-but-wrong* figure: a fabricated mark, a wrapped P&L, a summary dressed
up as an exact book. A dash you can see beats a number you can't trust. So the
whole surface follows one rule: **make the unknown visible; never smooth it over.**

This isn't a feature you toggle; it's a constraint every panel obeys.

## The six ways it shows up

**1. Dash the unknown, don't guess.** No source for the mark, index, or funding?
The cell reads `—`, not an invented number. An empty field is honest; a fabricated
one is a landmine. (`index —`, `funding —` until a real feed lands.)

**2. Label the estimate.** The client-derived mark is written `~mark N (mid)` —
the leading `~` and the `(mid)` say, out loud, "this is my estimate from the
book, not the exchange's authoritative mark." The user never mistakes a
client-side approximation for exchange truth.

**3. Aggregate ≠ exact.** In the log-time heatmap, near rows are the exact book;
far rows are *time-weighted liquidity profiles* over a window. Far rows are dimmed
and marked ("~10 s window") — they say "this is roughly how the book looked",
never "here is a book you can trade against." A fisheye's aggregated deep column
resolves a cursor to its *inner* (touch-side) price — the least-surprising choice,
not a hidden one.

**4. Hard-block, don't soft-warn.** An order over the fat-finger notional cap is
*refused outright* — no dismissable "are you sure?" dialog. A warning you can
click through is a fat-finger fill waiting to happen; a hard block is a wall. The
guard is the notional ceiling, and it's absolute.

**5. Withhold on overflow — never show a wrapped figure.** Position and uPnL fold
with checked i64 arithmetic. An oversized fill that would wrap `Net`/`Cost` is
*rejected* (state left unchanged); a uPnL computation that would overflow returns
"unavailable" rather than a wrapped, plausible-but-false dollar figure. A missing
number is safe; a wrong one that looks right is not.

**6. Reject malformed input at the boundary.** The public market-data feed is
untrusted (spec 4-cast §10.4 draws that boundary). A trade with an unknown
aggressor side is *skipped*, not coerced into a buy; a level with a garbage price
is dropped, not folded. A lying frame must not corrupt the picture.

## The cost it removes

It removes an entire *class* of failure — "the terminal told me X, I bet on X, X
was fake." Every uncertainty is surfaced as a dash, a `~`, a dim, or a refusal,
so the trader always knows which part of the screen is exact, which is an
estimate, and which is simply unknown.

## The through-line

**Trustworthy under pressure beats impressive.** The instinct to fill every field
with *something* is exactly the instinct to resist: a terminal that fabricates
once can't be trusted at all. Colour carries meaning (green = live/filled, red =
down, amber = degraded), the honesty tells (`—`, `~`, dim, hard-block) carry
*confidence*, and nothing is coloured or numbered "to look nice."

---

*Related: this is the same discipline behind the adversarial audit (position
overflow → withhold, malformed HL side → skip) and the compression design
(`compression.md`: far rows marked aggregate, cursor resolves to the inner
edge). Even the incumbents concede the limit honesty forces — Bookmap's own docs
note hidden orders "can only be displayed after execution has taken place";
intent is unprovable from book data, so the honest move is to show what's known
and mark what isn't.*
