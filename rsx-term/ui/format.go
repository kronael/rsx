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
