package ui

import (
	"fmt"
	"strconv"
	"strings"
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

// parseRaw is fmtDec's inverse: it turns a human-decimal input string into a
// raw i64 at `dec` decimals — "0.010001"@6 → 10001, "5"@4 → 50000, "5.5"@4 →
// 55000, ".5"@4 → 5000. So the trader types the price/qty they *read* off the
// ladder, and the raw i64 wire value is reconstructed here. Returns false on a
// second dot, a non-digit, an empty value, or more fractional digits than the
// instrument has (which would silently drop precision). dec<=0 means integer
// input (raw == the typed integer), so a tick-1 / no-decimals symbol is
// unchanged.
func parseRaw(s string, dec int) (int64, bool) {
	if s == "" {
		return 0, false
	}
	intPart, fracPart := s, ""
	if i := strings.IndexByte(s, '.'); i >= 0 {
		intPart, fracPart = s[:i], s[i+1:]
	}
	if strings.IndexByte(fracPart, '.') >= 0 { // a second dot
		return 0, false
	}
	if dec <= 0 {
		if fracPart != "" {
			return 0, false // no fractional part at zero decimals
		}
		return parseDigits(intPart)
	}
	if len(fracPart) > dec {
		return 0, false // more precision than the instrument carries
	}
	for len(fracPart) < dec {
		fracPart += "0"
	}
	return parseDigits(intPart + fracPart)
}

// safeMul multiplies two i64 and reports whether it stayed in range. A notional
// (price × size) of absurd manual inputs can overflow i64; showing the wrapped
// (often negative) result as a real number would break the terminal's "never
// fabricate a figure" rule, so the caller dashes it instead.
func safeMul(a, b int64) (int64, bool) {
	if a == 0 || b == 0 {
		return 0, true
	}
	c := a * b
	if c/b != a {
		return 0, false
	}
	return c, true
}

// parseDigits parses a pure-digit string to int64, rejecting empties and any
// non-digit (so "1.2", "-1", "1e3" are all invalid — parseRaw handles the dot).
func parseDigits(s string) (int64, bool) {
	if s == "" {
		return 0, false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return 0, false
		}
	}
	v, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		return 0, false
	}
	return v, true
}

// strWidth is the widest string in ss, floored at min — for right-aligning a
// column of formatted (decimal) values so it stays rigid instead of going
// ragged. Prices/qtys are ASCII so len() is the visible width.
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
