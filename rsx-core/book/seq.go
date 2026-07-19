package book

// SeqTracker detects gaps in a monotonic (non-strict) marketdata feed
// sequence. Per specs/2/49-webproto.md: "Gap detection: if u jumps > 1,
// re-subscribe."
type SeqTracker struct {
	last uint64
	seen bool
}

// Observe records seq and reports whether it constitutes a gap: a
// previous seq was seen AND seq > last+1. A seq <= last (duplicate, or a
// shared-seq frame — multiple frames can legitimately carry the same
// engine height) is never a gap. The very first Observe() call is never
// a gap.
func (t *SeqTracker) Observe(seq uint64) bool {
	gap := t.seen && seq > t.last+1
	if !t.seen || seq > t.last {
		t.last = seq
	}
	t.seen = true
	return gap
}

// Reset forgets all state, as if Observe had never been called.
func (t *SeqTracker) Reset() {
	t.last = 0
	t.seen = false
}

// ResetTo resets state and primes last=seq, so the next Observe(seq+1)
// is clean. Used after a re-subscribe delivers a fresh snapshot.
func (t *SeqTracker) ResetTo(seq uint64) {
	t.last = seq
	t.seen = true
}
