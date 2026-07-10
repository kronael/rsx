package book

import (
	"time"

	"rsx-term/wire"
)

// Heatmap is the multi-resolution "text Bookmap" state: a log-time vertical
// axis of price-space rows. The BOTTOM of the display is live — the current
// book renders every frame (LiveFold, it never scrolls away) over a short ring
// of per-bin (~100ms) rows. Above the live block each row aggregates an
// exponentially longer window on the fixed farSpans schedule (10s, 60s, 120s,
// 300s, 600s, then hours), so a fixed row count spans now→hours: recent is
// fine-grained, distant is coarse, and the top rows barely move.
//
// Rows store PRICE-SPACE profiles (LevelSample/TradeSample by raw px), not
// screen columns: the fisheye mapping happens at render time against the
// CURRENT anchor, so a recenter re-aligns the whole picture and far windows
// aggregate independently of any axis drift. Near/live rows carry the exact
// book (per-level, plus level ages from an AgeSource); far rows carry a
// time-weighted liquidity profile of the WHOLE book — where liquidity
// concentrated and how deep the book ran, not individual orders.
//
// It is pure state — no rendering, no clock: the caller supplies bin
// timestamps. See ui/stream.go for the renderer.
type Heatmap struct {
	tick    int64
	anchor  int64 // sticky centre in DOUBLED price units (bid+ask); 0 = unset
	liveCap int
	live    []Row // sealed live bins, oldest first
	tiers   []*tier
}

// LevelSample is one price bucket of a row's liquidity profile. Size and
// Count are exact for live rows and time-weighted means for far rows. AgeNs
// is how long the level has persisted (an AgeSource reading, live rows only;
// 0 = fresh or unknown).
type LevelSample struct {
	Px    int64
	Size  int64
	Count int32
	Side  int8 // -1 bid, +1 ask (dominant side for far rows)
	AgeNs int64
}

// TradeSample is executed trade-flow at one price within a row's window —
// the co-equal second layer next to resting liquidity. Qty sums every print
// at that price by the same aggressor within the window.
type TradeSample struct {
	Px   int64
	Qty  int64
	Side wire.Side // aggressor
}

// Row is one display row: a liquidity profile plus the trade-flow that
// printed inside its wall-clock window [FromNs, ToNs]. Span is the row's
// schedule width (0 = a live per-bin row).
type Row struct {
	Levels []LevelSample
	Trades []TradeSample
	FromNs int64
	ToNs   int64
	Span   time.Duration
}

// farSpans is the founder-decided log-time schedule: the aggregation window
// of each far row, nearest (just above the live block) first. Rows beyond
// the schedule each span farSpanMax.
var farSpans = []time.Duration{
	10 * time.Second,
	60 * time.Second,
	120 * time.Second,
	300 * time.Second,
	600 * time.Second,
}

// farSpanMax is the span of every far row past the end of farSpans ("then
// hours").
const farSpanMax = time.Hour

// FarSpan returns the aggregation window of far row i (0 = nearest to live).
func FarSpan(i int) time.Duration {
	if i < 0 {
		i = 0
	}
	if i < len(farSpans) {
		return farSpans[i]
	}
	return farSpanMax
}

// tier accumulates one far row's window: time-weighted (ms-weighted) sums
// per price bucket, plus the window's trade-flow. It seals into a Row once
// coveredMs reaches the span, then feeds the next tier and restarts.
type tier struct {
	span      time.Duration
	fromNs    int64
	toNs      int64
	coveredMs int64
	levels    map[int64]*levelAcc
	buys      map[int64]int64
	sells     map[int64]int64
}

// levelAcc is one price bucket's time-weighted accumulation (all in
// value × milliseconds, so an hour of a large book stays well inside i64).
type levelAcc struct {
	sizeMs  int64
	countMs int64
	bidMs   int64
	askMs   int64
}

func newTier(span time.Duration) *tier {
	return &tier{
		span:   span,
		levels: make(map[int64]*levelAcc),
		buys:   make(map[int64]int64),
		sells:  make(map[int64]int64),
	}
}

// linearTicks is how many price ticks around the touch render one-to-one
// (1 tick per column) before the fisheye starts aggregating.
const linearTicks = 8

// hysteresisTicks is how far (in ticks) the mid may drift before the anchor
// re-centres — the same stationary-ladder rule the DOM view uses.
const hysteresisTicks = 8

// NewHeatmap builds an empty heatmap. tick is the smallest raw price
// increment (<=0 falls back to 1). Configure sizes the row structure.
func NewHeatmap(tick int64) *Heatmap {
	if tick <= 0 {
		tick = 1
	}
	return &Heatmap{tick: tick}
}

// Configure sets the live-ring capacity and the number of far rows. The live
// ring is preserved (trimmed to the new cap); far tiers are rebuilt only when
// their count changes (their windows restart — acceptable for a resize).
func (h *Heatmap) Configure(liveCap, farRows int) {
	if liveCap < 1 {
		liveCap = 1
	}
	if farRows < 0 {
		farRows = 0
	}
	h.liveCap = liveCap
	if len(h.live) > liveCap {
		h.live = h.live[len(h.live)-liveCap:]
	}
	if len(h.tiers) != farRows {
		h.tiers = make([]*tier, farRows)
		for i := range h.tiers {
			h.tiers[i] = newTier(FarSpan(i))
		}
	}
}

// Tick returns the price increment the fisheye maps against.
func (h *Heatmap) Tick() int64 { return h.tick }

// LiveCap returns the live ring's row capacity.
func (h *Heatmap) LiveCap() int { return h.liveCap }

// FarRows returns the number of far (aggregated) rows.
func (h *Heatmap) FarRows() int { return len(h.tiers) }

// Anchor returns the sticky centre in doubled price units (0 before the
// first Ingest).
func (h *Heatmap) Anchor() int64 { return h.anchor }

// MidPx returns the anchored centre as a single price, or 0 before the first
// Ingest.
func (h *Heatmap) MidPx() int64 { return h.anchor / 2 }

// Ingest seals one live bin: it re-anchors on the current mid (with
// hysteresis), folds the whole book + the bin's trades + level ages into an
// exact price-space Row covering [fromNs, toNs], and pushes it into the live
// ring. A row expiring off the live ring cascades into the far tiers.
func (h *Heatmap) Ingest(bids, asks []wire.Level, trades []TapeEntry, ages AgeSource, fromNs, toNs int64) {
	h.updateAnchor(bids, asks)
	row := LiveFold(bids, asks, trades, ages, fromNs, toNs)
	h.live = append(h.live, row)
	for len(h.live) > h.liveCap {
		expired := h.live[0]
		h.live = h.live[1:]
		h.cascade(0, expired)
	}
}

// cascade folds an expired row into tier i, sealing and promoting the tier's
// window upward whenever it completes. Past the last tier the history rolls
// off the top.
func (h *Heatmap) cascade(i int, row Row) {
	if i >= len(h.tiers) {
		return
	}
	t := h.tiers[i]
	t.fold(row)
	if t.coveredMs >= t.span.Milliseconds() {
		sealed := t.snapshot()
		h.tiers[i] = newTier(t.span)
		h.cascade(i+1, sealed)
	}
}

// fold accumulates one row (time-weighted by its wall-clock duration) into
// the tier's window.
func (t *tier) fold(row Row) {
	ms := (row.ToNs - row.FromNs) / int64(time.Millisecond)
	if ms < 1 {
		ms = 1
	}
	for _, l := range row.Levels {
		acc := t.levels[l.Px]
		if acc == nil {
			acc = &levelAcc{}
			t.levels[l.Px] = acc
		}
		acc.sizeMs += l.Size * ms
		acc.countMs += int64(l.Count) * ms
		if l.Side < 0 {
			acc.bidMs += ms
		} else {
			acc.askMs += ms
		}
	}
	for _, tr := range row.Trades {
		if tr.Side == wire.Sell {
			t.sells[tr.Px] += tr.Qty
		} else {
			t.buys[tr.Px] += tr.Qty
		}
	}
	if t.coveredMs == 0 || row.FromNs < t.fromNs {
		t.fromNs = row.FromNs
	}
	if row.ToNs > t.toNs {
		t.toNs = row.ToNs
	}
	t.coveredMs += ms
}

// snapshot renders the tier's current accumulation as a Row: time-weighted
// mean size/count per price bucket (a liquidity profile, not exact orders),
// dominant side, summed trade-flow. Empty tiers yield an empty Row carrying
// only the span, so the display keeps its fixed grid.
func (t *tier) snapshot() Row {
	row := Row{FromNs: t.fromNs, ToNs: t.toNs, Span: t.span}
	if t.coveredMs == 0 {
		return row
	}
	for px, acc := range t.levels {
		size := acc.sizeMs / t.coveredMs
		if size == 0 {
			continue // present too briefly to register at this resolution
		}
		count := int32(acc.countMs / t.coveredMs)
		if count < 1 {
			count = 1
		}
		side := int8(-1)
		if acc.askMs > acc.bidMs {
			side = 1
		}
		row.Levels = append(row.Levels, LevelSample{Px: px, Size: size, Count: count, Side: side})
	}
	for px, qty := range t.buys {
		row.Trades = append(row.Trades, TradeSample{Px: px, Qty: qty, Side: wire.Buy})
	}
	for px, qty := range t.sells {
		row.Trades = append(row.Trades, TradeSample{Px: px, Qty: qty, Side: wire.Sell})
	}
	return row
}

// LiveFold builds an exact price-space Row from the current book: every level
// of both sides (the whole book — the fisheye's edge columns aggregate the
// depths at render), the bin's trade prints, and each level's persistence age
// (ages may be nil). Shared by Ingest and by the renderer's always-live "now"
// row.
func LiveFold(bids, asks []wire.Level, trades []TapeEntry, ages AgeSource, fromNs, toNs int64) Row {
	row := Row{FromNs: fromNs, ToNs: toNs}
	row.Levels = make([]LevelSample, 0, len(bids)+len(asks))
	fold := func(levels []wire.Level, side int8) {
		for _, l := range levels {
			if l.Qty <= 0 {
				continue
			}
			count := int32(l.Count)
			if count < 1 {
				count = 1
			}
			age := int64(0)
			if ages != nil {
				age = ages.AgeNs(l.Px, toNs)
			}
			row.Levels = append(row.Levels, LevelSample{Px: l.Px, Size: l.Qty, Count: count, Side: side, AgeNs: age})
		}
	}
	fold(bids, -1)
	fold(asks, +1)
	for _, tr := range trades {
		row.Trades = append(row.Trades, TradeSample{Px: tr.Px, Qty: tr.Qty, Side: tr.Side})
	}
	return row
}

// Rows returns every display row except the live "now" row, top-first: the
// farthest tier's window down through the nearest, then the live ring oldest
// to newest. Empty tiers and a short live ring still yield rows/gaps the
// renderer pads, keeping the grid fixed.
func (h *Heatmap) Rows() []Row {
	out := make([]Row, 0, len(h.tiers)+len(h.live))
	for i := len(h.tiers) - 1; i >= 0; i-- {
		out = append(out, h.tiers[i].snapshot())
	}
	out = append(out, h.live...)
	return out
}

// LiveLen returns how many live bins the ring currently holds.
func (h *Heatmap) LiveLen() int { return len(h.live) }

// updateAnchor moves the sticky centre only when the current doubled-mid
// drifts beyond hysteresisTicks of the anchor, so the axis stays put
// tick-to-tick.
func (h *Heatmap) updateAnchor(bids, asks []wire.Level) {
	if len(bids) == 0 || len(asks) == 0 {
		return
	}
	cur := bids[0].Px + asks[0].Px // 2*mid, exact
	if h.anchor == 0 {
		h.anchor = cur
		return
	}
	band := 2 * hysteresisTicks * h.tick
	d := cur - h.anchor
	if d < 0 {
		d = -d
	}
	if d > band {
		h.anchor = cur
	}
}

// FisheyeCol maps a raw price to its fisheye column for the given anchor
// (doubled price units), tick, and column count. Columns 0..width/2-1 are the
// bid side (col width/2-1 = the touch, col 0 = deepest, clamped so the whole
// book lands on-screen); columns width/2..width-1 mirror for asks. Returns
// false when the axis is unanchored or px sits exactly on the mid (the
// spread gap).
func FisheyeCol(px, anchor, tick int64, width int) (int, bool) {
	if anchor == 0 || width < 2 {
		return 0, false
	}
	if tick <= 0 {
		tick = 1
	}
	half := width / 2
	num := 2*px - anchor
	if num == 0 {
		return 0, false
	}
	if num > 0 { // ask side, right of centre
		ticks := (num + 2*tick - 1) / (2 * tick) // ceil to whole ticks
		col := half + fisheyeOffset(ticks) - 1
		if col > width-1 {
			col = width - 1
		}
		return col, true
	}
	ticks := (-num + 2*tick - 1) / (2 * tick)
	col := half - fisheyeOffset(ticks)
	if col < 0 {
		col = 0
	}
	return col, true
}

// FisheyePx is FisheyeCol's inverse for interaction (cursor, click): the raw
// price at the inner (touch-side) edge of the column's bucket, so a click on
// an aggregated deep column lands on its nearest — least surprising — price.
// Returns 0 when unanchored.
func FisheyePx(col int, anchor, tick int64, width int) int64 {
	if anchor == 0 || width < 2 {
		return 0
	}
	if tick <= 0 {
		tick = 1
	}
	half := width / 2
	mid := anchor / 2
	if col >= half { // ask side
		ticks := offsetTicks(col - half + 1)
		return mid + ticks*tick
	}
	ticks := offsetTicks(half - col)
	px := mid - ticks*tick
	if px < tick {
		px = tick
	}
	return px
}

// offsetTicks maps a column offset from the centre (>=1) back to the
// inner-edge whole-tick distance — the inverse of fisheyeOffset's schedule.
func offsetTicks(offset int) int64 {
	if offset < 1 {
		offset = 1
	}
	if offset <= linearTicks {
		return int64(offset)
	}
	extra := int64(offset - linearTicks)
	// First tick of the extra-th compressed bucket: linear zone + the
	// triangular ticks consumed by the previous buckets + 1.
	return int64(linearTicks) + (extra-1)*extra/2 + 1
}

// fisheyeOffset maps a whole-tick distance from the touch (>=1) to a column
// offset from the centre. The first linearTicks are one-to-one (1 tick per
// column); beyond that a triangular schedule compresses ever-wider tick
// ranges into single columns — the sqrt-like fisheye that keeps the touch
// sharp and aggregates the depths.
func fisheyeOffset(ticks int64) int {
	if ticks < 1 {
		ticks = 1
	}
	if ticks <= linearTicks {
		return int(ticks)
	}
	return linearTicks + compressTicks(ticks-linearTicks)
}

// compressTicks maps an extra tick distance (beyond the linear zone) to an
// added column offset: the k-th extra column spans k ticks (cumulative
// k(k+1)/2), so far levels aggregate progressively wider price ranges.
func compressTicks(extra int64) int {
	n := 1
	for int64(n)*int64(n+1)/2 < extra {
		n++
	}
	return n
}
