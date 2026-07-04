//! Hierarchical occupancy bitmap for O(1) next-occupied-level lookup.
//!
//! One bit per compression slot: set = "this level is non-empty". A
//! small tree of `u64` summary words sits on top (level 1 = one bit per
//! level-0 word, level 2 = one bit per level-1 word, ...), so
//! find-next-set and find-prev-set skip empty regions by reading a
//! handful of summary words instead of scanning the whole slot array.
//!
//! Why this shape (cache locality, the founder's priority):
//! - Word-dense: 64 slots per `u64`, so one cache line (8 words) covers
//!   512 slots. Skipping a gap touches summary words, not the 24-byte
//!   `PriceLevel` array (which is ~192x larger per slot).
//! - Contiguous: each level is one `Vec<u64>` — no pointer chasing, no
//!   heap-scattered nodes. A find touches ~`depth` words (depth is 3 for
//!   a ~120k-slot book), each in a hot summary cache line.
//! - `set`/`clear` are O(depth) and only walk UP while a word flips
//!   empty<->non-empty, so a cancel deep in the book is a couple of word
//!   writes, never a scan.
//!
//! The compression map is a SAWTOOTH (index is not globally
//! price-monotonic across zones), so best-bid/best-ask are NOT a single
//! find over the whole bitmap — the caller walks the per-zone price-
//! ordered sub-ranges (`Orderbook::price_asc`) and does a bounded find
//! within each. See `book.rs`.

/// Set-bit occupancy over `n` slots with `u64` summary levels.
pub struct Occupancy {
    /// `levels[0]` is the slot bitmap; `levels[k+1]` summarizes
    /// `levels[k]` (bit `w` set iff word `w` of `levels[k]` is
    /// non-zero). The top level is always exactly one word.
    levels: Vec<Vec<u64>>,
    n: u32,
}

impl Occupancy {
    /// Build an empty bitmap covering `n` slots.
    pub fn new(n: u32) -> Self {
        let mut levels: Vec<Vec<u64>> = Vec::new();
        let mut count = n as usize;
        loop {
            let words = count.div_ceil(64).max(1);
            levels.push(vec![0u64; words]);
            if words == 1 {
                break;
            }
            count = words;
        }
        Self { levels, n }
    }

    #[inline]
    pub fn len(&self) -> u32 {
        self.n
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Mark slot `i` non-empty. Propagates a summary bit upward only
    /// while a word transitions 0 -> non-zero.
    #[inline]
    pub fn set(&mut self, i: u32) {
        let mut i = i as usize;
        let depth = self.levels.len();
        for lvl in 0..depth {
            let word = i >> 6;
            let bit = i & 63;
            let prev = self.levels[lvl][word];
            self.levels[lvl][word] = prev | (1u64 << bit);
            if prev != 0 {
                // Word was already non-empty: parent bit already set.
                break;
            }
            i = word;
        }
    }

    /// Mark slot `i` empty. Clears a summary bit upward only while a
    /// word transitions non-zero -> 0.
    #[inline]
    pub fn clear(&mut self, i: u32) {
        let mut i = i as usize;
        let depth = self.levels.len();
        for lvl in 0..depth {
            let word = i >> 6;
            let bit = i & 63;
            self.levels[lvl][word] &= !(1u64 << bit);
            if self.levels[lvl][word] != 0 {
                // Word still has other bits: parent stays set.
                break;
            }
            i = word;
        }
    }

    /// Lowest set slot `>= from`, or `None`. O(depth): climb until a
    /// summary word has a candidate, then descend to the exact bit.
    #[inline]
    pub fn find_next(&self, from: u32) -> Option<u32> {
        if from >= self.n {
            return None;
        }
        let depth = self.levels.len();
        let mut lvl = 0usize;
        let mut idx = from as usize;
        let found;
        loop {
            let word = idx >> 6;
            if word >= self.levels[lvl].len() {
                return None;
            }
            let masked =
                self.levels[lvl][word] & (u64::MAX << (idx & 63));
            if masked != 0 {
                found = word * 64 + masked.trailing_zeros() as usize;
                break;
            }
            lvl += 1;
            if lvl >= depth {
                return None;
            }
            idx = word + 1;
        }
        // Descend: `found` is a bit index at `lvl` = word index at
        // `lvl-1`; take the lowest set bit each step down.
        let mut pos = found;
        while lvl > 0 {
            lvl -= 1;
            let w = self.levels[lvl][pos];
            pos = pos * 64 + w.trailing_zeros() as usize;
        }
        debug_assert!((pos as u32) < self.n);
        Some(pos as u32)
    }

    /// Highest set slot `<= from`, or `None`. Symmetric to `find_next`.
    #[inline]
    pub fn find_prev(&self, from: u32) -> Option<u32> {
        if self.n == 0 {
            return None;
        }
        let from = from.min(self.n - 1) as usize;
        let depth = self.levels.len();
        let mut lvl = 0usize;
        let mut idx = from;
        let found;
        loop {
            let word = idx >> 6;
            let off = idx & 63;
            let mask = if off == 63 {
                u64::MAX
            } else {
                (1u64 << (off + 1)) - 1
            };
            let masked = self.levels[lvl][word] & mask;
            if masked != 0 {
                found =
                    word * 64 + (63 - masked.leading_zeros() as usize);
                break;
            }
            if word == 0 {
                return None;
            }
            lvl += 1;
            if lvl >= depth {
                return None;
            }
            idx = word - 1;
        }
        let mut pos = found;
        while lvl > 0 {
            lvl -= 1;
            let w = self.levels[lvl][pos];
            pos = pos * 64 + (63 - w.leading_zeros() as usize);
        }
        Some(pos as u32)
    }

    /// Lowest set slot in `[lo, hi)`, or `None`.
    #[inline]
    pub fn find_first_in(&self, lo: u32, hi: u32) -> Option<u32> {
        if lo >= hi {
            return None;
        }
        self.find_next(lo).filter(|&b| b < hi)
    }

    /// Highest set slot in `[lo, hi)`, or `None`.
    #[inline]
    pub fn find_last_in(&self, lo: u32, hi: u32) -> Option<u32> {
        if lo >= hi {
            return None;
        }
        self.find_prev(hi - 1).filter(|&b| b >= lo)
    }
}

#[cfg(test)]
#[path = "occupancy_test.rs"]
mod occupancy_test;
