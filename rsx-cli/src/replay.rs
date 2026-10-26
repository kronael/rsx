use crate::check::Tally;
use crate::ledger::Ledger;
use crate::ledger::OpenOrder;
use crate::ledger::OrderKey;
use rsx_matching::config::BootConfig;
use rsx_matching::recovery::recover;
use rsx_matching::recovery::Recovered;
use rustc_hash::FxHashMap;
use serde_json::json;
use serde_json::Value;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[cfg(test)]
#[path = "replay_test.rs"]
mod replay_test;

const CAPACITY_MARGIN: u32 = 1024;

/// What replaying a stream into an `rsx_book::book::Orderbook` shows.
#[derive(Default)]
pub struct Replayed {
    /// The replayed book holds what the WAL says rests.
    pub matches: Tally,
    /// No resting buy is priced at or above a resting sell.
    pub crossed: Tally,
    /// Allocated slots = free slots + active orders.
    pub slab: Tally,
}

impl Replayed {
    pub fn merge(&mut self, other: Replayed) {
        self.matches.merge(&other.matches);
        self.crossed.merge(&other.crossed);
        self.slab.merge(&other.slab);
    }
}

pub fn book_capacity(ledger: &Ledger) -> u32 {
    ledger.peak_resting.saturating_add(CAPACITY_MARGIN)
}

/// Rebuilds the book of `boot.config.symbol_id` the way the matching engine
/// boots: its checkpoint, then the WAL after it. A panic inside the book
/// (an exhausted slab, an event buffer overflow) stops the process, as it
/// stops the engine's boot.
pub fn recover_book(boot: BootConfig, wal_dir: &Path) -> Result<Recovered, String> {
    let Some(dir) = wal_dir.to_str() else {
        return Err(format!("wal dir {} is not UTF-8", wal_dir.display()));
    };
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos() as u64;
    recover(boot, dir, now_ns).map_err(|error| error.to_string())
}

fn resting_orders(recovered: &Recovered) -> Vec<(OrderKey, OpenOrder)> {
    let book = &recovered.book;
    let mut resting = Vec::new();
    for handle in 0..book.orders.len() {
        let slot = book.orders.get(handle);
        if !slot.is_active() {
            continue;
        }
        resting.push((
            (slot.user_id, slot.order_id_hi, slot.order_id_lo),
            OpenOrder {
                side: slot.side,
                price: slot.price.0,
                remaining: slot.remaining_qty.0,
            },
        ));
    }
    resting
}

fn finding(ledger: &Ledger, message: &str, fields: Value) -> Value {
    let mut value = json!({ "stream_id": ledger.stream_id(), "message": message });
    if let (Value::Object(map), Value::Object(fields)) = (&mut value, fields) {
        map.extend(fields);
    }
    value
}

fn order_fields(key: OrderKey) -> Value {
    json!({ "user_id": key.0, "order_id": format!("{:016x}{:016x}", key.1, key.2) })
}

/// Judges the replayed `recovered` book of the stream behind `ledger`. The
/// ledger and the book must have read the stream up to the same seq. The
/// second value says why the WAL comparison could not run.
pub fn judge(ledger: &Ledger, recovered: &Recovered) -> (Replayed, Option<String>) {
    let mut replayed = Replayed::default();
    let resting = resting_orders(recovered);

    let book = &recovered.book;
    replayed.slab.checked += 1;
    let allocated = book.orders.len();
    let free = book.orders.free_count();
    let active = resting.len() as u32;
    if allocated != free + active {
        replayed.slab.add_violation(finding(
            ledger,
            "allocated slots differ from free slots plus active orders",
            json!({ "allocated": allocated, "free": free, "active": active }),
        ));
    }

    replayed.crossed.checked += 1;
    let best_bid = resting
        .iter()
        .filter(|(_, order)| order.side == 0)
        .map(|(_, order)| order.price)
        .max();
    let best_ask = resting
        .iter()
        .filter(|(_, order)| order.side == 1)
        .map(|(_, order)| order.price)
        .min();
    if let (Some(bid), Some(ask)) = (best_bid, best_ask) {
        if bid >= ask {
            replayed.crossed.add_violation(finding(
                ledger,
                "a resting buy is priced at or above a resting sell",
                json!({ "best_bid": bid, "best_ask": ask }),
            ));
        }
    }

    let skip = compare_with_ledger(ledger, recovered, &resting, &mut replayed.matches);
    (replayed, skip)
}

fn compare_with_ledger(
    ledger: &Ledger,
    recovered: &Recovered,
    resting: &[(OrderKey, OpenOrder)],
    matches: &mut Tally,
) -> Option<String> {
    let first_seq = ledger.first_seq?;
    let checkpoint_seq = recovered.checkpoint_wal_seq;
    if checkpoint_seq > ledger.last_seq {
        matches.checked += 1;
        matches.add_violation(finding(
            ledger,
            "the checkpoint is ahead of the WAL",
            json!({ "checkpoint_wal_seq": checkpoint_seq, "wal_last_seq": ledger.last_seq }),
        ));
        return None;
    }
    let is_covered =
        ledger.is_anchored() || (checkpoint_seq > 0 && first_seq <= checkpoint_seq + 1);
    if !is_covered {
        return Some(format!(
            "stream {} starts at seq {} and no checkpoint reaches it",
            ledger.stream_id(),
            first_seq
        ));
    }
    if !ledger.is_cycle_complete() {
        return Some(format!(
            "stream {} ends inside a match cycle; run again",
            ledger.stream_id()
        ));
    }
    matches.checked += 1;
    let mut book: FxHashMap<OrderKey, &OpenOrder> = FxHashMap::default();
    for (key, order) in resting {
        book.insert(*key, order);
    }
    let mut wal: Vec<(OrderKey, OpenOrder)> = ledger.open_orders().collect();
    wal.sort_unstable_by_key(|(key, _)| *key);
    for (key, expected) in &wal {
        match book.remove(key) {
            None => matches.add_violation(finding(
                ledger,
                "rests in the WAL, missing from the replayed book",
                order_fields(*key),
            )),
            Some(actual) => {
                if actual.side != expected.side
                    || actual.price != expected.price
                    || actual.remaining != expected.remaining
                {
                    let mut fields = order_fields(*key);
                    if let Value::Object(map) = &mut fields {
                        map.insert(
                            "wal".to_string(),
                            json!({
                                "side": expected.side,
                                "price": expected.price,
                                "remaining": expected.remaining,
                            }),
                        );
                        map.insert(
                            "book".to_string(),
                            json!({
                                "side": actual.side,
                                "price": actual.price,
                                "remaining": actual.remaining,
                            }),
                        );
                    }
                    matches.add_violation(finding(
                        ledger,
                        "the replayed book differs from the WAL",
                        fields,
                    ));
                }
            }
        }
    }
    if ledger.is_anchored() {
        let mut extra: Vec<OrderKey> = book.keys().copied().collect();
        extra.sort_unstable();
        for key in extra {
            matches.add_violation(finding(
                ledger,
                "rests in the replayed book, not in the WAL",
                order_fields(key),
            ));
        }
    }
    None
}
