package main

import "math"

// Quote math. Pure integer functions on raw fixed-point prices:
// half-spread = ref * spread_bps / 10000, level offsets widen by
// half-spread/2 per level, bids floor to the tick, asks ceil to it.
// Per-level size is uniform by default; the opt-in "walls" depth profile
// (demo scenes) shapes it thin-at-touch / deep-at-walls via depthQty.

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

// topQuote is the best bid and ask a cycle placed for one symbol; a
// non-positive side was not placed.
type topQuote struct {
	bid, ask int64
}

// jitterShift limits a tick-aligned shift of the whole ladder so its top bid
// stays at least one tick below prev's top ask and its top ask at least one
// tick above prev's top bid: prev may still rest when the new ladder lands,
// and a touch there locks the book. base is the unshifted top. Both limits
// are tick multiples and the lower never exceeds the upper, so the result is
// tick-aligned and satisfies both.
func jitterShift(shift, tick int64, base, prev topQuote) int64 {
	if prev.ask > 0 {
		shift = min(shift, prev.ask-tick-base.bid)
	}
	if prev.bid > 0 {
		shift = max(shift, prev.bid+tick-base.ask)
	}
	return shift
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

// wallRamp is the "walls" depth profile shape: per-level size numerators
// over wallRampDen. Level 0 (the touch) quotes half the base size; far
// levels build up to 6x — a ~12x touch-to-wall span, enough to cross
// several intensity tiers of a log-scaled liquidity heatmap.
var wallRamp = []int64{1, 2, 4, 7, 12}

const wallRampDen = 2

// depthQty returns the per-level order size in raw lot units for a
// depth profile. "" (the default) is the uniform orderQty ladder the
// production maker quotes; "walls" thins the touch and deepens the far
// levels so the book reads like a real ladder: small size where price
// trades, size walls behind it. Levels beyond the ramp hold the last
// (deepest) multiplier. Always at least one lot, lot-aligned.
func depthQty(profile string, qtyPerLevel, lot int64, level int) int64 {
	base := orderQty(qtyPerLevel, lot)
	if profile != "walls" {
		return base
	}
	if lot < 1 {
		lot = 1
	}
	q := base * wallRamp[min(level, len(wallRamp)-1)] / wallRampDen
	q = q / lot * lot
	if q < lot {
		q = lot
	}
	return q
}

// floorTick rounds v down to the nearest multiple of tick.
func floorTick(v, tick int64) int64 {
	return v / tick * tick
}

// ceilTick rounds v up to the nearest multiple of tick.
func ceilTick(v, tick int64) int64 {
	return (v + tick - 1) / tick * tick
}

// catalogMinNotional is a symbol's minimum order notional in risk's raw
// price x qty unit. The catalog's own value wins; without it (the catalog's
// config source is down) the 1 USD preset minimum, 10^(price_decimals +
// qty_decimals), is used. 0 when neither is known.
func catalogMinNotional(catalog *int64, priceDecimals, qtyDecimals int) int64 {
	if catalog != nil && *catalog > 0 {
		return *catalog
	}
	places := priceDecimals + qtyDecimals
	if places < 1 || places > 18 {
		return 0
	}
	floor := int64(1)
	for range places {
		floor *= 10
	}
	return floor
}

// minQty raises qty to the smallest lot multiple whose notional at price
// reaches the symbol's minimum, so risk does not reject the order as too
// small. A qty already at the minimum is returned unchanged. The result
// saturates at the largest lot multiple an int64 holds.
func minQty(qty, price int64, spec symbolSpec) int64 {
	if spec.minNotional <= 0 || price <= 0 {
		return qty
	}
	lot := max(spec.lot, 1)
	// Smallest raw qty with price*qty >= minNotional is ceil(min/price);
	// round it up to a lot multiple.
	raw := (spec.minNotional + price - 1) / price
	if raw < 0 {
		raw = math.MaxInt64
	}
	lots := (raw + lot - 1) / lot
	if lots < 0 || lots > math.MaxInt64/lot {
		lots = math.MaxInt64 / lot
	}
	return max(qty, lots*lot)
}
