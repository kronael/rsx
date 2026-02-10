//! Mark price data structures. MARK.md §2.

use rsx_dxs::records::PayloadPreamble;

/// WAL wire format for mark price events.
/// Prefix(16) + 4 + 4 + 8 + 8 + 4 + 4 + 16(pad) = 64 bytes
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct MarkPriceEvent {
    pub preamble: PayloadPreamble,
    pub symbol_id: u32,
    pub _pad0: u32,
    pub mark_price: i64,
    pub timestamp_ns: u64,
    pub source_mask: u32,
    pub source_count: u32,
    pub _pad1: [u8; 16],
}

/// A single price update from one exchange source.
#[derive(Debug, Clone, Copy)]
pub struct SourcePrice {
    pub source_id: u8,
    pub price: i64,
    pub timestamp_ns: u64,
}

/// Per-symbol aggregation state. MARK.md §2.
pub struct SymbolMarkState {
    pub sources: [Option<SourcePrice>; 8],
    pub mark_price: i64,
    pub source_mask: u32,
    pub source_count: u8,
}

impl SymbolMarkState {
    pub fn new() -> Self {
        Self {
            sources: [None; 8],
            mark_price: 0,
            source_mask: 0,
            source_count: 0,
        }
    }
}

impl Default for SymbolMarkState {
    fn default() -> Self {
        Self::new()
    }
}
