//! Mark price data structures. MARK.md §2.

pub type MarkPriceEvent = rsx_dxs::records::MarkPriceRecord;

pub type SymbolMap = std::collections::HashMap<String, u32>;

/// A single price update from one exchange source.
#[derive(Debug, Clone, Copy)]
pub struct SourcePrice {
    pub symbol_id: u32,
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
