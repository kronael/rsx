use crate::types::FillEvent;
use rustc_hash::FxHashMap;
use tracing::debug;
use tracing::info;

pub struct ReplicaState {
    buffered_fills: Vec<FxHashMap<u64, FillEvent>>,
    last_tips: Vec<u64>,
}

impl ReplicaState {
    pub fn new(max_symbols: usize) -> Self {
        Self {
            buffered_fills: vec![FxHashMap::default(); max_symbols],
            last_tips: vec![0u64; max_symbols],
        }
    }

    pub fn buffer_fill(&mut self, fill: FillEvent) {
        let sym_idx = fill.symbol_id as usize;
        if sym_idx >= self.buffered_fills.len() {
            return;
        }
        self.buffered_fills[sym_idx].insert(fill.seq, fill);
    }

    pub fn apply_tip(&mut self, symbol_id: u32, tip: u64) {
        let sym_idx = symbol_id as usize;
        if sym_idx >= self.last_tips.len() {
            return;
        }
        self.last_tips[sym_idx] = tip;
    }

    pub fn drain_fills_up_to_tip(
        &mut self,
        symbol_id: u32,
    ) -> Vec<FillEvent> {
        let sym_idx = symbol_id as usize;
        if sym_idx >= self.buffered_fills.len() {
            return Vec::new();
        }
        let tip = self.last_tips[sym_idx];
        let buffer = &mut self.buffered_fills[sym_idx];
        let mut fills: Vec<_> = buffer
            .iter()
            .filter(|(&seq, _)| seq <= tip)
            .map(|(_, f)| f.clone())
            .collect();
        fills.sort_by_key(|f| f.seq);
        for f in &fills {
            buffer.remove(&f.seq);
        }
        debug!(
            symbol_id,
            tip,
            count = fills.len(),
            "drained buffered fills up to tip"
        );
        fills
    }

    pub fn drain_all_up_to_tips(&mut self) -> Vec<FillEvent> {
        let mut all_fills = Vec::new();
        for symbol_id in 0..self.buffered_fills.len() {
            let fills =
                self.drain_fills_up_to_tip(symbol_id as u32);
            all_fills.extend(fills);
        }
        all_fills
    }

    pub fn buffered_count(&self, symbol_id: u32) -> usize {
        let sym_idx = symbol_id as usize;
        if sym_idx >= self.buffered_fills.len() {
            return 0;
        }
        self.buffered_fills[sym_idx].len()
    }

    pub fn total_buffered(&self) -> usize {
        self.buffered_fills.iter().map(|m| m.len()).sum()
    }

    pub fn last_tip(&self, symbol_id: u32) -> u64 {
        let sym_idx = symbol_id as usize;
        if sym_idx >= self.last_tips.len() {
            return 0;
        }
        self.last_tips[sym_idx]
    }
}

pub struct ReplicaPromotion {
    pub fills_applied: usize,
    pub final_tips: Vec<u64>,
}

pub fn promote_replica(
    state: &mut ReplicaState,
) -> ReplicaPromotion {
    let fills = state.drain_all_up_to_tips();
    let count = fills.len();
    let final_tips = state.last_tips.clone();
    info!(
        fills_applied = count,
        "promotion: applying buffered fills up to last tips"
    );
    ReplicaPromotion {
        fills_applied: count,
        final_tips,
    }
}
