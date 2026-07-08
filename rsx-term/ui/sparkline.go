package ui

// sparkRamp is the Unicode block ramp used to render a magnitude, lowest to
// highest glyph.
var sparkRamp = []rune("▁▂▃▄▅▆▇█")

// sparkline renders samples as a compact string of block glyphs scaled to
// the samples' own min/max — the "watch the latency live" strip. A flat
// window (min == max) renders the mid glyph for every sample instead of
// dividing by zero. Empty input renders "".
func sparkline(samples []int64) string {
	if len(samples) == 0 {
		return ""
	}
	lo, hi := samples[0], samples[0]
	for _, v := range samples[1:] {
		if v < lo {
			lo = v
		}
		if v > hi {
			hi = v
		}
	}
	span := hi - lo
	out := make([]rune, len(samples))
	for i, v := range samples {
		if span == 0 {
			out[i] = sparkRamp[len(sparkRamp)/2]
			continue
		}
		idx := (v - lo) * int64(len(sparkRamp)-1) / span
		out[i] = sparkRamp[idx]
	}
	return string(out)
}
