# Three nonlinear compressions — how a book and its history fit a text grid

An order book is the live list of resting bids and asks at each price; a
liquidity heatmap shows how that book *and its recent history* look at a glance.
The problem the terminal has to solve is a mismatch of scale: a perps book has
hundreds of price levels, a useful history is minutes to hours, and a terminal
gives you maybe 80×40 character cells. Two axes, each with orders of magnitude
more data than there are cells.

The naive answer — one cell per tick, one row per tick of the clock — shows a
few dollars of book over a few seconds and throws everything else away. The fix
is the same idea applied three times, once per axis: **replace a linear mapping
with a nonlinear one that magnifies what matters and compresses the periphery.**

## Time — a logarithmic cadence, not a uniform scroll

**Problem.** A uniform time axis (1 row = one ~100 ms bin) shows ~4 seconds in
40 rows. The book streams up and off the top in seconds — the whole picture
"flies away" — and any context older than a few seconds is simply gone.

**Fix.** Rows run on a fixed *log-cadence schedule*. The bottom of the display
is **live**: a short ring of exact per-bin (~100 ms) rows, redrawn every frame
so the current book never scrolls away. Above the live block, each row
aggregates an exponentially longer window — 10 s, 60 s, 120 s, 300 s, 600 s,
then hours. A fixed row count therefore spans *now → hours ago*: recent history
is fine-grained, distant history is coarse, and the top rows barely move. Near
rows carry the **exact** book (per level); far rows carry a **time-weighted
liquidity profile** of the whole book — *where liquidity concentrated and how
deep it ran over that window*, not individual orders.

**Cost it removes.** The whole session's shape fits on one screen, the live edge
is always current, and "how did we get here" is answerable without a chart in
another window.

## Price — a fisheye, not a linear ladder

**Problem.** Hundreds of levels, but the action is at the touch. A linear price
axis (1 cell = 1 tick) shows a sliver near the mid and can't show the deep walls
at all.

**Fix.** A **fisheye**: near the touch the axis is 1 tick per cell (full
resolution — this is where you quote and trade); past a small linear zone,
successive cells aggregate an increasing number of ticks (a triangular
schedule), summing their size. Deep levels compress into the edges; the whole
book lands on screen. The axis is anchored on the mid with hysteresis, so it
re-centres only when the mid genuinely drifts rather than jittering every tick.

**Cost it removes.** Near-touch precision *and* whole-book depth in one width —
the two things traders otherwise juggle in two separate views.

## Size — a logarithmic colour ramp on a stable basis

**Problem.** Resting sizes are heavy-tailed. One whale on a linear intensity
scale saturates its cell and pushes everything else to black — the structure you
came to read disappears behind the outlier.

**Fix.** Map size to intensity on a **log/√ scale**, normalised against a
**stable basis** that rises instantly but decays slowly (rather than the
per-frame maximum, which makes the whole view flicker as the biggest order comes
and goes). Sizes render as a handful of tiers, never as exact numbers.

**Cost it removes.** The size *distribution* reads — you see the shape of the
book, not just its single largest order — and it doesn't strobe.

## The through-line

**Three nonlinear compressions, one per axis** — time (log cadence), price
(fisheye), size (log colour). Each keeps full fidelity where a trader looks
(recent, near-touch, typical) and gracefully coarsens the periphery (old, deep,
whale). The fixed terminal grid becomes a focus-plus-context lens on the market:
a small window that nonetheless implies the whole.

## Honesty is part of the compression

Compression that lies is worse than none. The design keeps the coarsening
*visible*: far-time rows are marked as aggregate windows ("~10 s window", dimmed),
never dressed up as an exact book you could trade against; the fisheye's
aggregated deep columns resolve a click/cursor to their *inner* (touch-side)
price, the least-surprising one; sizes are shown as tiers, not fabricated exact
figures. The viewer always knows which part of the picture is exact and which is
a summary.

---

*Prior art: the liquidity-heatmap-over-time is Bookmap's contribution, inverted
here from a GPU bitmap into a character grid; the fisheye is the classic
focus-plus-context / "degree of interest" idea (Furnas, 1986); log colour for
heavy-tailed magnitudes is standard scientific-visualisation practice. What's
new is doing all three in text, keyboard-only, so the same compressed model can
later drive a bitmap frontend unchanged. See `book/heatmap.go` (the ring, the
far-tier cascade, `FisheyeCol`/`FisheyePx`) and `compression`'s companion note
`glyph-bank.md` for how a compressed cell is actually drawn.*
