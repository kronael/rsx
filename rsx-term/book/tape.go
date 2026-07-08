package book

import "rsx-term/wire"

// TapeEntry is one public trade print.
type TapeEntry struct {
	Side wire.Side
	Px   int64
	Qty  int64
}

// MaxTrades caps the retained tape length.
const MaxTrades = 50

// Tape is the public trade tape, newest print first.
type Tape struct {
	entries []TapeEntry
}

// Push prepends e, dropping the oldest entry once len exceeds MaxTrades.
func (t *Tape) Push(e TapeEntry) {
	t.entries = append([]TapeEntry{e}, t.entries...)
	if len(t.entries) > MaxTrades {
		t.entries = t.entries[:MaxTrades]
	}
}

// Entries returns the tape, newest first.
func (t *Tape) Entries() []TapeEntry {
	return t.entries
}

// Last returns the most recent print, or false if the tape is empty.
func (t *Tape) Last() (TapeEntry, bool) {
	if len(t.entries) == 0 {
		return TapeEntry{}, false
	}
	return t.entries[0], true
}
