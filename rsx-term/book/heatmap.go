package book

import "rsx-term/wire"

// Heatmap is a time-binned "text Bookmap": a fixed-length ring of rows, one row
// per ~100ms bin. Each row holds one Cell per fisheye price column, capturing
// the resting size + order-count on each side plus any trades that printed in
// that bin. Newest row is last (rendered at the bottom); older rows age toward
// the front and fall off the ring once it is full. It is pure — no rendering,
// no clock: the caller supplies the bin timestamp to Ingest. See ui/stream.go
// for the renderer.
//
// Price axis (fisheye, mid-centred): columns 0..half-1 are the bid side (left,
// col half-1 = the touch, col 0 = deepest); columns half..width-1 are the ask
// side (right, col half = the touch, col width-1 = deepest). The innermost
// linearTicks price ticks map one-to-one to columns; beyond that, columns each
// span a widening tick range (a triangular fisheye), so near-touch reads at
// full 1-tick resolution and deep levels aggregate many ticks into one bucket
// (sizes summed, orders counted). The anchor recenters on the mid with
// hysteresis, mirroring the static ladder (recenterLadder).
type Heatmap struct {
	width  int
	height int
	tick   int64
	// anchor is the sticky centre in DOUBLED price units (best bid + best ask =
	// 2*mid). Doubling keeps a 1-tick spread's half-tick centre exact, so the
	// bid and ask touches sit symmetric about the centre. 0 = uninitialised.
	anchor int64
	rows   []Row
}

// Cell is one fisheye price bucket within one time bin. Resting liquidity is
// single-sided (a column is either bid or ask), so one Size/Count/Side triple
// suffices; trades are tracked per aggressor because either side can print at
// any column.
type Cell struct {
	Size      int64 // resting size summed into this bucket
	Count     int32 // resting orders counted into this bucket (>=1 per level)
	Side      int8  // -1 bid, +1 ask, 0 empty
	BuyTrade  int64 // buy-aggressor qty printed in this bin at this column
	SellTrade int64 // sell-aggressor qty printed in this bin at this column
}

// Row is one time bin: a full column of Cells plus the bin timestamp (for the
// news-rail time axis) and the anchor it was mapped against.
type Row struct {
	Cells  []Cell
	BinTs  int64
	Anchor int64
}

// linearTicks is how many price ticks around the touch render one-to-one
// (1 tick per column) before the fisheye starts aggregating.
const linearTicks = 8

// hysteresisTicks is how far (in ticks) the mid may drift before the anchor
// re-centres — the same stationary-ladder rule the DOM view uses.
const hysteresisTicks = 8

// NewHeatmap builds an empty heatmap with width price columns and height time
// rows. tick is the smallest raw price increment (<=0 falls back to 1 so the
// axis always advances).
func NewHeatmap(width, height int, tick int64) *Heatmap {
	if tick <= 0 {
		tick = 1
	}
	if width < 2 {
		width = 2
	}
	if height < 1 {
		height = 1
	}
	return &Heatmap{width: width, height: height, tick: tick}
}

// Width returns the number of price columns.
func (h *Heatmap) Width() int { return h.width }

// Height returns the ring length (rows / time bins).
func (h *Heatmap) Height() int { return h.height }

// MidPx returns the anchored centre as a single (un-doubled) price, or 0 before
// the first Ingest.
func (h *Heatmap) MidPx() int64 { return h.anchor / 2 }

// Rows returns the live rows oldest-first (index 0 is the top / oldest, the
// last element is the newest / bottom). Shorter than Height until the ring
// fills. The slice is owned by the Heatmap — read, don't mutate.
func (h *Heatmap) Rows() []Row { return h.rows }

// Ingest folds one time bin: it re-anchors on the current mid (with
// hysteresis), buckets every bid/ask level into a fresh row via the fisheye,
// records the bin's trades, and pushes the row as the newest, dropping the
// oldest once the ring is full. binTs is the bin's wall-clock timestamp (ns),
// supplied by the caller so the fold stays clock-free and testable.
func (h *Heatmap) Ingest(bids, asks []wire.Level, trades []TapeEntry, binTs int64) {
	h.updateAnchor(bids, asks)

	row := Row{Cells: make([]Cell, h.width), BinTs: binTs, Anchor: h.anchor}
	for _, l := range bids {
		h.foldLevel(row.Cells, l, -1)
	}
	for _, l := range asks {
		h.foldLevel(row.Cells, l, +1)
	}
	for _, t := range trades {
		col, ok := h.colFor(t.Px)
		if !ok {
			continue
		}
		if t.Side == wire.Sell {
			row.Cells[col].SellTrade += t.Qty
		} else {
			row.Cells[col].BuyTrade += t.Qty
		}
	}

	h.rows = append(h.rows, row)
	if len(h.rows) > h.height {
		h.rows = h.rows[len(h.rows)-h.height:]
	}
}

// foldLevel sums one resting level into its fisheye column.
func (h *Heatmap) foldLevel(cells []Cell, l wire.Level, side int8) {
	col, ok := h.colFor(l.Px)
	if !ok {
		return
	}
	cnt := int32(l.Count)
	if cnt < 1 {
		cnt = 1
	}
	c := &cells[col]
	c.Size += l.Qty
	c.Count += cnt
	c.Side = side
}

// updateAnchor moves the sticky centre only when the current doubled-mid drifts
// beyond hysteresisTicks of the anchor, so the axis stays put tick-to-tick.
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

// colFor maps a raw price to its fisheye column, or false when the axis is
// unanchored or the price sits exactly on the mid (the centre gap / spread).
func (h *Heatmap) colFor(px int64) (int, bool) {
	if h.anchor == 0 {
		return 0, false
	}
	half := h.width / 2
	num := 2*px - h.anchor
	if num == 0 {
		return 0, false
	}
	if num > 0 { // ask side, right of centre
		ticks := (num + 2*h.tick - 1) / (2 * h.tick) // ceil to whole ticks
		col := half + fisheyeOffset(ticks) - 1
		if col > h.width-1 {
			col = h.width - 1
		}
		return col, true
	}
	ticks := (-num + 2*h.tick - 1) / (2 * h.tick)
	col := half - fisheyeOffset(ticks)
	if col < 0 {
		col = 0
	}
	return col, true
}

// fisheyeOffset maps a whole-tick distance from the touch (>=1) to a column
// offset from the centre. The first linearTicks are one-to-one (1 tick per
// column); beyond that a triangular schedule compresses ever-wider tick ranges
// into single columns — the sqrt-like fisheye that keeps the touch sharp and
// aggregates the depths.
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
