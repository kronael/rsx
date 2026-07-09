package ui

import "fmt"

// fmtNs renders a nanosecond duration adaptively with integer math (never
// floats): "340 ns", "9.6 µs", "1.28 ms". Mirrors rsx-tui render.rs fmt_ns.
func fmtNs(ns int64) string {
	if ns < 1_000 {
		return fmt.Sprintf("%d ns", ns)
	}
	if ns < 1_000_000 {
		return fmt.Sprintf("%d.%d µs", ns/1_000, (ns%1_000)/100)
	}
	return fmt.Sprintf("%d.%02d ms", ns/1_000_000, (ns%1_000_000)/10_000)
}

// digitWidth is the character width of v printed in base 10, including a
// leading '-' for negatives.
func digitWidth(v int64) int {
	return len(fmt.Sprintf("%d", v))
}

// colWidth is the widest digitWidth across vals, floored at min. Used to
// right-align a price/qty/notional column to the widest value currently on
// screen so the column stays rigid instead of going ragged the instant a
// value crosses a digit boundary (e.g. 9998 -> 10004).
func colWidth(min int, vals ...int64) int {
	w := min
	for _, v := range vals {
		if d := digitWidth(v); d > w {
			w = d
		}
	}
	return w
}

// fmtDec renders a raw fixed-point i64 as a human decimal with dec places
// (raw / 10^dec) — the display-boundary conversion (CLAUDE.md: convert only
// here; the wire stays raw i64). dec<=0 returns the plain integer, so a
// tick-1 / no-decimals symbol reads as before.
func fmtDec(raw int64, dec int) string {
	if dec <= 0 {
		return fmt.Sprintf("%d", raw)
	}
	neg := raw < 0
	if neg {
		raw = -raw
	}
	scale := int64(1)
	for i := 0; i < dec; i++ {
		scale *= 10
	}
	s := fmt.Sprintf("%d.%0*d", raw/scale, dec, raw%scale)
	if neg {
		s = "-" + s
	}
	return s
}

// strWidth is the widest string in ss, floored at min — for right-aligning a
// column of formatted (decimal) values, the string-width sibling of colWidth.
// Prices/qtys are ASCII so len() is the visible width.
func strWidth(min int, ss ...string) int {
	w := min
	for _, s := range ss {
		if len(s) > w {
			w = len(s)
		}
	}
	return w
}

// clamp restricts v to [lo, hi].
func clamp(v, lo, hi int) int {
	if v < lo {
		return lo
	}
	if v > hi {
		return hi
	}
	return v
}
