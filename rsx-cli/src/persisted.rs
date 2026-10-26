use crate::check::Check;
use crate::check::Tally;
use crate::check::POSITIONS;
use crate::check::TIPS;
use crate::ledger::Ledger;
use rsx_risk::Position;
use rustc_hash::FxHashMap;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "persisted_test.rs"]
mod persisted_test;

/// What risk has persisted in Postgres: positions and the tip of every
/// shard per symbol.
pub struct Persisted {
    pub positions: FxHashMap<(u32, u32), Position>,
    /// `(instance_id, symbol_id, last_seq)` rows of `tips`.
    pub tips: Vec<(u32, u32, u64)>,
    /// Per symbol, the lowest `last_seq` any shard has persisted.
    pub lowest_tip: FxHashMap<u32, u64>,
}

impl Persisted {
    pub fn new(positions: FxHashMap<(u32, u32), Position>, tips: Vec<(u32, u32, u64)>) -> Self {
        let mut lowest_tip: FxHashMap<u32, u64> = FxHashMap::default();
        for (_, symbol_id, last_seq) in &tips {
            lowest_tip
                .entry(*symbol_id)
                .and_modify(|lowest| *lowest = (*lowest).min(*last_seq))
                .or_insert(*last_seq);
        }
        Persisted {
            positions,
            tips,
            lowest_tip,
        }
    }
}

fn position_json(position: &Position) -> Value {
    json!({
        "long_qty": position.long_qty,
        "short_qty": position.short_qty,
        "long_entry_cost": position.long_entry_cost,
        "short_entry_cost": position.short_entry_cost,
        "realized_pnl": position.realized_pnl,
        "last_fill_seq": position.last_fill_seq,
        "version": position.version,
    })
}

fn is_same_position(left: &Position, right: &Position) -> bool {
    left.long_qty == right.long_qty
        && left.short_qty == right.short_qty
        && left.long_entry_cost == right.long_entry_cost
        && left.short_entry_cost == right.short_entry_cost
        && left.realized_pnl == right.realized_pnl
        && left.last_fill_seq == right.last_fill_seq
        && left.version == right.version
}

/// Position = sum of fills. Each persisted position row is compared with the
/// position `rsx_risk::Position::apply_fill` yields over the WAL fills up to
/// the row's `last_fill_seq`; and every position with a fill at or below the
/// lowest persisted tip of its symbol must have a row that reflects it.
/// Only streams that start at seq 1 can be summed.
pub fn check_positions(persisted: &Persisted, ledgers: &[Ledger]) -> Check {
    let mut by_symbol: FxHashMap<u32, &Ledger> = FxHashMap::default();
    for ledger in ledgers {
        if ledger.has_order_flow() && ledger.is_anchored() {
            by_symbol.entry(ledger.stream_id()).or_insert(ledger);
        }
    }
    let mut tally = Tally::default();
    let mut unsummed: BTreeSet<u32> = BTreeSet::new();
    let mut keys: Vec<(u32, u32)> = persisted.positions.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let (user_id, symbol_id) = key;
        let (Some(ledger), Some(row)) = (by_symbol.get(&symbol_id), persisted.positions.get(&key))
        else {
            unsummed.insert(symbol_id);
            continue;
        };
        tally.checked += 1;
        let summed = ledger
            .replayed_positions
            .get(&key)
            .cloned()
            .unwrap_or_else(|| Position::new(user_id, symbol_id));
        if !is_same_position(&summed, row) {
            tally.add_violation(json!({
                "message": "persisted position differs from the sum of the WAL fills",
                "user_id": user_id,
                "symbol_id": symbol_id,
                "wal": position_json(&summed),
                "postgres": position_json(row),
            }));
        }
    }
    let mut symbols: Vec<u32> = by_symbol.keys().copied().collect();
    symbols.sort_unstable();
    for symbol_id in symbols {
        let Some(ledger) = by_symbol.get(&symbol_id) else {
            continue;
        };
        let mut owed: Vec<(&(u32, u32), &u64)> = ledger.must_be_persisted.iter().collect();
        owed.sort_unstable();
        for (key, seq) in owed {
            tally.checked += 1;
            let is_reflected = persisted
                .positions
                .get(key)
                .is_some_and(|row| row.last_fill_seq >= *seq);
            if !is_reflected {
                tally.add_violation(json!({
                    "message": "no persisted position reflects a fill risk has already tipped past",
                    "user_id": key.0,
                    "symbol_id": key.1,
                    "fill_seq": seq,
                    "lowest_tip": persisted.lowest_tip.get(&key.1),
                }));
            }
        }
    }
    let mut check = Check::from_tally(Some(4), POSITIONS, "positions", tally);
    if !unsummed.is_empty() {
        let symbols: Vec<String> = unsummed.iter().map(u32::to_string).collect();
        check.detail.push_str(&format!(
            "; no WAL stream from seq 1 for symbols {}",
            symbols.join(",")
        ));
    }
    check
}

/// Tips never pass the ME stream they follow: a shard's `last_seq` for a
/// symbol is at most the last seq on the symbol's WAL. `streams` pairs a
/// stream id with the last seq on disk.
pub fn check_tips(persisted: &Persisted, streams: &[(u32, u64)]) -> Check {
    let last_by_stream: FxHashMap<u32, u64> = streams.iter().copied().collect();
    let mut tally = Tally::default();
    let mut rows = persisted.tips.clone();
    rows.sort_unstable();
    for (instance_id, symbol_id, last_seq) in rows {
        let Some(wal_last_seq) = last_by_stream.get(&symbol_id) else {
            continue;
        };
        tally.checked += 1;
        if last_seq > *wal_last_seq {
            tally.add_violation(json!({
                "message": "tip is past the last seq of the ME WAL",
                "instance_id": instance_id,
                "symbol_id": symbol_id,
                "tip": last_seq,
                "wal_last_seq": wal_last_seq,
            }));
        }
    }
    Check::from_tally(Some(5), TIPS, "tips", tally)
}

/// The sum of one symbol's funding payments for one interval.
pub struct IntervalSum {
    pub symbol_id: u32,
    pub interval_id: u64,
    /// Shards that recorded the interval as settled.
    pub shards_settled: u64,
    pub total: i128,
}

/// Funding is zero-sum per symbol and interval. Each shard rounds its own
/// payments down to a whole unit, so the sum over `k` settling shards falls
/// in `-(k - 1) ..= 0`; one shard sums to exactly zero. An interval some
/// shard has not settled yet is not examined; the second value counts them.
pub fn tally_funding(sums: &[IntervalSum], shards: u64) -> (Tally, u64) {
    let mut tally = Tally::default();
    let mut unsettled = 0;
    for sum in sums {
        if sum.shards_settled < shards {
            unsettled += 1;
            continue;
        }
        tally.checked += 1;
        let rounding = sum.shards_settled.max(1) as i128 - 1;
        if sum.total > 0 || sum.total < -rounding {
            tally.add_violation(json!({
                "message": "funding payments do not sum to zero",
                "symbol_id": sum.symbol_id,
                "interval_id": sum.interval_id,
                "shards_settled": sum.shards_settled,
                "total": sum.total.to_string(),
            }));
        }
    }
    (tally, unsettled)
}

/// Shard lock keys and the number of backends holding each. A key held by
/// more than one backend breaks the single main per shard.
pub fn tally_locks(holders_per_key: &[(i64, i64)]) -> Tally {
    let mut tally = Tally {
        checked: holders_per_key.len() as u64,
        ..Tally::default()
    };
    for (key, holders) in holders_per_key {
        if *holders > 1 {
            tally.add_violation(json!({
                "message": "advisory lock held by more than one backend",
                "lock_key": key,
                "holders": holders,
            }));
        }
    }
    tally
}
