//! Aggregation logic. MARK.md §4, §6.

use crate::types::MarkPriceEvent;
use crate::types::SourcePrice;
use crate::types::SymbolMarkState;

/// 10 seconds in nanoseconds.
pub const STALENESS_NS: u64 = 10_000_000_000;

/// Max sources per symbol.
pub const MAX_SOURCES: usize = 8;

/// Compute the median of a sorted slice of prices.
/// Even count: picks the lower median (left of center).
pub fn median(sorted: &[i64]) -> i64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        // even: average of two middle, but spec says
        // "lower median" for even count
        // Two sources: "median is avg" per test name
        // Even count > 2: "picks lower median"
        if n == 2 {
            // avg rounded down (integer division)
            (sorted[0] + sorted[1]) / 2
        } else {
            sorted[n / 2 - 1]
        }
    }
}

/// Compute bitmask of fresh (non-stale) sources.
pub fn compute_mask(
    state: &SymbolMarkState,
    now_ns: u64,
    staleness_ns: u64,
) -> u32 {
    let mut mask: u32 = 0;
    for sp in state.sources.iter().flatten() {
        if now_ns.saturating_sub(sp.timestamp_ns)
            < staleness_ns
        {
            mask |= 1 << sp.source_id;
        }
    }
    mask
}

/// Result of an aggregation: Some(event) if should publish.
pub fn aggregate(
    state: &mut SymbolMarkState,
    update: SourcePrice,
    now_ns: u64,
    symbol_id: u32,
) -> Option<MarkPriceEvent> {
    let sid = update.source_id as usize;
    if sid >= MAX_SOURCES {
        return None;
    }

    state.sources[sid] = Some(update);

    reaggregate(state, now_ns, symbol_id, STALENESS_NS)
}

/// Re-aggregate from current sources. Returns event if
/// there are fresh sources.
pub fn reaggregate(
    state: &mut SymbolMarkState,
    now_ns: u64,
    symbol_id: u32,
    staleness_ns: u64,
) -> Option<MarkPriceEvent> {
    let mut fresh: Vec<i64> = Vec::with_capacity(MAX_SOURCES);
    for sp in state.sources.iter().flatten() {
        if now_ns.saturating_sub(sp.timestamp_ns)
            < staleness_ns
        {
            fresh.push(sp.price);
        }
    }

    state.source_mask =
        compute_mask(state, now_ns, staleness_ns);
    state.source_count = fresh.len() as u8;

    match fresh.len() {
        0 => None,
        1 => {
            state.mark_price = fresh[0];
            Some(make_event(state, symbol_id, now_ns))
        }
        _ => {
            fresh.sort();
            state.mark_price = median(&fresh);
            Some(make_event(state, symbol_id, now_ns))
        }
    }
}

fn make_event(
    state: &SymbolMarkState,
    symbol_id: u32,
    now_ns: u64,
) -> MarkPriceEvent {
    MarkPriceEvent {
        seq: 0,
        ts_ns: now_ns,
        symbol_id,
        _pad0: 0,
        mark_price: rsx_types::Price(state.mark_price),
        source_mask: state.source_mask,
        source_count: state.source_count as u32,
        _pad1: [0; 24],
    }
}

/// Staleness sweep for one symbol. MARK.md §4.
/// Returns Some(event) if mark price changed due to
/// a source becoming stale.
pub fn sweep_stale(
    state: &mut SymbolMarkState,
    now_ns: u64,
    symbol_id: u32,
) -> Option<MarkPriceEvent> {
    // Check if any previously-fresh source is now stale
    let old_mask = state.source_mask;
    let new_mask = compute_mask(state, now_ns, STALENESS_NS);

    if new_mask == old_mask {
        return None;
    }

    // A source became stale, re-aggregate
    reaggregate(state, now_ns, symbol_id, STALENESS_NS)
}

/// Same as aggregate but with configurable staleness.
pub fn aggregate_with_staleness(
    state: &mut SymbolMarkState,
    update: SourcePrice,
    now_ns: u64,
    symbol_id: u32,
    staleness_ns: u64,
) -> Option<MarkPriceEvent> {
    let sid = update.source_id as usize;
    if sid >= MAX_SOURCES {
        return None;
    }

    state.sources[sid] = Some(update);
    reaggregate(state, now_ns, symbol_id, staleness_ns)
}

/// Same as sweep_stale but with configurable staleness.
pub fn sweep_stale_with_staleness(
    state: &mut SymbolMarkState,
    now_ns: u64,
    symbol_id: u32,
    staleness_ns: u64,
) -> Option<MarkPriceEvent> {
    let old_mask = state.source_mask;
    let new_mask = compute_mask(state, now_ns, staleness_ns);
    if new_mask == old_mask {
        return None;
    }
    reaggregate(state, now_ns, symbol_id, staleness_ns)
}
