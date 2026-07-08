package ui

import (
	"fmt"

	"rsx-term/book"
)

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

// fmtNsOrDash renders "—" for an unmeasured leg (book.NsUnknown), else fmtNs.
// Never fabricate a 0 for a missing measurement.
func fmtNsOrDash(ns int64) string {
	if ns == book.NsUnknown {
		return "—"
	}
	return fmtNs(ns)
}
