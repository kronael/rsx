package book

import "sort"

// NsUnknown marks a latency leg as not measured. The UI renders "—" for
// this; never fabricate a 0.
const NsUnknown int64 = -1

// Sample is one round-trip latency measurement. TotalNs is always a real
// client measurement; the three leg fields are NsUnknown on the live
// wire (webproto-49 has no gateway-side timing stamps; only the offline
// mock/demo supplies real splits).
type Sample struct {
	TotalNs    int64
	NetNs      int64
	InternalNs int64
	EngineNs   int64
}

// MaxSamples caps the rolling latency window.
const MaxSamples = 128

// Window is a rolling FIFO window of TotalNs values, capped at
// MaxSamples.
type Window struct {
	samples []int64
}

// Add appends ns to the window, dropping the oldest sample once len
// exceeds MaxSamples. Negative values (unmeasured) are ignored.
func (w *Window) Add(ns int64) {
	if ns < 0 {
		return
	}
	w.samples = append(w.samples, ns)
	if len(w.samples) > MaxSamples {
		w.samples = w.samples[len(w.samples)-MaxSamples:]
	}
}

// P50 returns sorted[len/2] of the window (matching the Rust App's
// parity — not an average of the two middle values), or false if empty.
func (w *Window) P50() (int64, bool) {
	if len(w.samples) == 0 {
		return 0, false
	}
	sorted := append([]int64(nil), w.samples...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i] < sorted[j] })
	return sorted[len(sorted)/2], true
}

// Min returns the smallest value in the window, or false if empty.
func (w *Window) Min() (int64, bool) {
	if len(w.samples) == 0 {
		return 0, false
	}
	m := w.samples[0]
	for _, v := range w.samples[1:] {
		if v < m {
			m = v
		}
	}
	return m, true
}
