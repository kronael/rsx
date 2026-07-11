package main

// Quote math. Pure integer functions on raw fixed-point prices.
// Ported faithfully from rsx-playground/market_maker.py's quote cycle:
// half-spread = ref * spread_bps / 10000, level offsets widen by
// half-spread/2 per level, bids floor to the tick, asks ceil to it.

// halfSpread returns the per-side distance from ref in raw price
// units for a given spread in basis points. At least one tick-less
// unit so a zero-bps or tiny-ref quote still has a non-zero width.
func halfSpread(ref, spreadBps int64) int64 {
	h := ref * spreadBps / 10000
	if h < 1 {
		return 1
	}
	return h
}

// levelStep is the extra offset added per depth level beyond the top.
func levelStep(half int64) int64 {
	s := half / 2
	if s < 1 {
		return 1
	}
	return s
}

// quote returns the tick-aligned bid and ask raw prices for one depth
// level around ref. level 0 is the top of book; deeper levels widen.
// Bid floors to the tick boundary, ask ceils to it, so both stay
// tick-aligned (the gateway rejects mis-aligned prices).
func quote(ref, spreadBps, tick int64, level int) (bid, ask int64) {
	if tick < 1 {
		tick = 1
	}
	half := halfSpread(ref, spreadBps)
	offset := half + int64(level)*levelStep(half)
	bid = floorTick(ref-offset, tick)
	ask = ceilTick(ref+offset, tick)
	return bid, ask
}

// orderQty returns the per-level order size in raw lot units: at least
// one lot, otherwise qtyPerLevel lots, aligned down to the lot size.
func orderQty(qtyPerLevel, lot int64) int64 {
	if lot < 1 {
		lot = 1
	}
	q := qtyPerLevel * lot
	if q < lot {
		q = lot
	}
	return q / lot * lot
}

// floorTick rounds v down to the nearest multiple of tick.
func floorTick(v, tick int64) int64 {
	return v / tick * tick
}

// ceilTick rounds v up to the nearest multiple of tick.
func ceilTick(v, tick int64) int64 {
	return (v + tick - 1) / tick * tick
}
