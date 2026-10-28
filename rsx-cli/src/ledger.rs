use crate::check::Tally;
use crate::persisted::Persisted;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_messages::BboRecord;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_risk::Position;
use rsx_types::FailureReason;
use rustc_hash::FxHashMap;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[cfg(test)]
#[path = "ledger_test.rs"]
mod ledger_test;

/// `(user_id, order_id_hi, order_id_lo)`; one matching engine stream holds
/// one symbol, so the key is unique within a stream.
pub type OrderKey = (u32, u64, u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Accepted,
    Resting,
    AwaitingDone,
    Done,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Accepted => "accepted",
            Phase::Resting => "resting",
            Phase::AwaitingDone => "awaiting_done",
            Phase::Done => "done",
        }
    }
}

struct Order {
    phase: Phase,
    accepted_seq: u64,
    insert_seq: u64,
    done_seq: u64,
    side: u8,
    price: i64,
    remaining: i64,
}

impl Order {
    fn new(accepted_seq: u64) -> Self {
        Order {
            phase: Phase::Accepted,
            accepted_seq,
            insert_seq: 0,
            done_seq: 0,
            side: 0,
            price: 0,
            remaining: 0,
        }
    }
}

/// A resting order as the WAL records it.
pub struct OpenOrder {
    pub side: u8,
    pub price: i64,
    pub remaining: i64,
}

/// The order lifecycle of one WAL stream, rebuilt record by record from the
/// records alone. Each check keeps its own [`Tally`].
///
/// A stream whose first record is seq 1 is anchored: every order in it has
/// its ORDER_ACCEPTED in the stream, so a record about an unknown order is a
/// violation. A stream that starts later has had its head pruned, and a
/// record about an unknown order is skipped.
pub struct Ledger<'a> {
    wal_dir: String,
    stream_id: u32,
    persisted: Option<&'a Persisted>,
    orders: FxHashMap<OrderKey, Order>,
    levels: FxHashMap<(u8, i64), BTreeMap<u64, OrderKey>>,
    in_flight: Option<OrderKey>,
    resting_now: u32,
    prev_seq: Option<u64>,
    pub first_seq: Option<u64>,
    pub last_seq: u64,
    pub accepted: u64,
    pub fills: u64,
    pub duplicates: u64,
    pub peak_resting: u32,
    pub intact: Tally,
    pub monotonic: Tally,
    pub fills_before_done: Tally,
    pub completion: Tally,
    pub fifo: Tally,
    pub bbo: Tally,
    pub replayed_positions: FxHashMap<(u32, u32), Position>,
    pub must_be_persisted: FxHashMap<(u32, u32), u64>,
}

fn order_hex(key: OrderKey) -> String {
    format!("{:016x}{:016x}", key.1, key.2)
}

fn order_fields(key: OrderKey) -> Value {
    json!({ "user_id": key.0, "order_id": order_hex(key) })
}

impl<'a> Ledger<'a> {
    pub fn new(wal_dir: &Path, stream_id: u32, persisted: Option<&'a Persisted>) -> Self {
        Ledger {
            wal_dir: wal_dir.to_string_lossy().into_owned(),
            stream_id,
            persisted,
            orders: FxHashMap::default(),
            levels: FxHashMap::default(),
            in_flight: None,
            resting_now: 0,
            prev_seq: None,
            first_seq: None,
            last_seq: 0,
            accepted: 0,
            fills: 0,
            duplicates: 0,
            peak_resting: 0,
            intact: Tally::default(),
            monotonic: Tally::default(),
            fills_before_done: Tally::default(),
            completion: Tally::default(),
            fifo: Tally::default(),
            bbo: Tally::default(),
            replayed_positions: FxHashMap::default(),
            must_be_persisted: FxHashMap::default(),
        }
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    pub fn is_anchored(&self) -> bool {
        self.first_seq == Some(1)
    }

    /// Whether the stream carries matching engine order flow.
    pub fn has_order_flow(&self) -> bool {
        self.accepted > 0 || self.fills > 0
    }

    /// False when the stream ends inside a match cycle: the last order was
    /// accepted or cancelled and its closing record is not on disk yet.
    pub fn is_cycle_complete(&self) -> bool {
        self.in_flight.is_none()
    }

    pub fn open_orders(&self) -> impl Iterator<Item = (OrderKey, OpenOrder)> + '_ {
        self.orders
            .iter()
            .filter(|(_, order)| order.phase == Phase::Resting)
            .map(|(key, order)| {
                (
                    *key,
                    OpenOrder {
                        side: order.side,
                        price: order.price,
                        remaining: order.remaining,
                    },
                )
            })
    }

    fn finding(&self, seq: u64, message: &str, fields: Value) -> Value {
        let mut map = Map::new();
        map.insert("wal_dir".to_string(), json!(self.wal_dir));
        map.insert("stream_id".to_string(), json!(self.stream_id));
        map.insert("seq".to_string(), json!(seq));
        map.insert("message".to_string(), json!(message));
        if let Value::Object(fields) = fields {
            map.extend(fields);
        }
        Value::Object(map)
    }

    fn decoded<T: Copy>(&mut self, seq: u64, record_type: u16, payload: &[u8]) -> Option<T> {
        self.intact.checked += 1;
        let record = decode_payload::<T>(payload);
        if record.is_none() {
            let message = format!(
                "record type {} has a {}-byte payload, shorter than its record",
                record_type,
                payload.len()
            );
            let violation = self.finding(seq, &message, json!({}));
            self.intact.add_violation(violation);
        }
        record
    }

    /// Feeds one record, in stream order.
    pub fn apply(&mut self, record_type: u16, payload: &[u8]) {
        let Some(seq) = extract_seq(payload) else {
            return;
        };
        self.note_seq(seq);
        match record_type {
            RECORD_ORDER_ACCEPTED => {
                if let Some(record) = self.decoded::<OrderAcceptedRecord>(seq, record_type, payload)
                {
                    self.on_accepted(seq, record);
                }
            }
            RECORD_FILL => {
                if let Some(record) = self.decoded::<FillRecord>(seq, record_type, payload) {
                    self.on_fill(seq, record);
                }
            }
            RECORD_ORDER_INSERTED => {
                if let Some(record) = self.decoded::<OrderInsertedRecord>(seq, record_type, payload)
                {
                    self.on_inserted(seq, record);
                }
            }
            RECORD_ORDER_CANCELLED => {
                if let Some(record) =
                    self.decoded::<OrderCancelledRecord>(seq, record_type, payload)
                {
                    self.on_cancelled(seq, record);
                }
            }
            RECORD_ORDER_DONE => {
                if let Some(record) = self.decoded::<OrderDoneRecord>(seq, record_type, payload) {
                    let key = (record.user_id, record.order_id_hi, record.order_id_lo);
                    self.complete(seq, key, "ORDER_DONE", true);
                }
            }
            RECORD_ORDER_FAILED => {
                if let Some(record) = self.decoded::<OrderFailedRecord>(seq, record_type, payload) {
                    self.on_failed(seq, record);
                }
            }
            RECORD_BBO => {
                if let Some(record) = self.decoded::<BboRecord>(seq, record_type, payload) {
                    self.on_bbo(seq, record);
                }
            }
            _ => {}
        }
    }

    fn note_seq(&mut self, seq: u64) {
        self.first_seq.get_or_insert(seq);
        self.last_seq = self.last_seq.max(seq);
        if let Some(previous) = self.prev_seq {
            self.monotonic.checked += 1;
            if seq <= previous {
                let message = format!("seq {} follows seq {}", seq, previous);
                let violation = self.finding(seq, &message, json!({ "previous_seq": previous }));
                self.monotonic.add_violation(violation);
            }
        }
        self.prev_seq = Some(seq);
    }

    fn add_completion_violation(&mut self, seq: u64, message: &str, fields: Value) {
        let violation = self.finding(seq, message, fields);
        self.completion.add_violation(violation);
    }

    fn on_accepted(&mut self, seq: u64, record: OrderAcceptedRecord) {
        let key = (record.user_id, record.order_id_hi, record.order_id_lo);
        self.accepted += 1;
        self.completion.checked += 1;
        let existing = self
            .orders
            .get(&key)
            .map(|order| (order.phase, order.accepted_seq));
        if let Some((phase, first_accepted_seq)) = existing {
            if phase != Phase::Done {
                let fields = json!({
                    "user_id": key.0,
                    "order_id": order_hex(key),
                    "first_accepted_seq": first_accepted_seq,
                });
                self.add_completion_violation(
                    seq,
                    "ORDER_ACCEPTED again before the order completed",
                    fields,
                );
                if phase == Phase::Resting {
                    self.leave_level(key);
                }
            }
        }
        self.orders.insert(key, Order::new(seq));
        self.in_flight = Some(key);
    }

    fn on_inserted(&mut self, seq: u64, record: OrderInsertedRecord) {
        let key = (record.user_id, record.order_id_hi, record.order_id_lo);
        let phase = self.orders.get(&key).map(|order| order.phase);
        match phase {
            Some(Phase::Accepted) => {}
            Some(other) => {
                let message = format!("ORDER_INSERTED for an order in phase {}", other.as_str());
                self.add_completion_violation(seq, &message, order_fields(key));
                return;
            }
            None => {
                if self.is_anchored() {
                    self.add_completion_violation(
                        seq,
                        "ORDER_INSERTED for an order with no ORDER_ACCEPTED",
                        order_fields(key),
                    );
                }
                self.orders.insert(key, Order::new(0));
            }
        }
        if let Some(order) = self.orders.get_mut(&key) {
            order.phase = Phase::Resting;
            order.insert_seq = seq;
            order.side = record.side;
            order.price = record.price.0;
            order.remaining = record.qty.0;
        }
        self.levels
            .entry((record.side, record.price.0))
            .or_default()
            .insert(seq, key);
        self.resting_now += 1;
        self.peak_resting = self.peak_resting.max(self.resting_now);
        if self.in_flight == Some(key) {
            self.in_flight = None;
        }
    }

    fn leave_level(&mut self, key: OrderKey) {
        let Some(order) = self.orders.get(&key) else {
            return;
        };
        let level_key = (order.side, order.price);
        if let Some(level) = self.levels.get_mut(&level_key) {
            level.remove(&order.insert_seq);
            if level.is_empty() {
                self.levels.remove(&level_key);
            }
        }
        self.resting_now = self.resting_now.saturating_sub(1);
    }

    fn on_fill(&mut self, seq: u64, record: FillRecord) {
        self.fills += 1;
        let taker = (
            record.taker_user_id,
            record.taker_order_id_hi,
            record.taker_order_id_lo,
        );
        let maker = (
            record.maker_user_id,
            record.maker_order_id_hi,
            record.maker_order_id_lo,
        );
        self.apply_to_positions(seq, &record);
        self.check_fill_precedes_done(seq, taker, "taker");
        self.check_fill_precedes_done(seq, maker, "maker");
        if self.is_anchored() && !self.orders.contains_key(&taker) {
            self.add_completion_violation(
                seq,
                "FILL for a taker with no ORDER_ACCEPTED",
                order_fields(taker),
            );
        }
        self.consume_maker(seq, &record, maker);
    }

    fn apply_to_positions(&mut self, seq: u64, record: &FillRecord) {
        let Some(persisted) = self.persisted else {
            return;
        };
        let symbol_id = record.symbol_id;
        let maker_side = if record.taker_side == 0 { 1 } else { 0 };
        let sides = [
            (record.taker_user_id, record.taker_side),
            (record.maker_user_id, maker_side),
        ];
        for (user_id, side) in sides {
            let key = (user_id, symbol_id);
            if let Some(row) = persisted.positions.get(&key) {
                if seq <= row.last_fill_seq {
                    self.replayed_positions
                        .entry(key)
                        .or_insert_with(|| Position::new(user_id, symbol_id))
                        .apply_fill(side, record.price.0, record.qty.0, seq);
                }
            }
            if persisted
                .lowest_tip
                .get(&symbol_id)
                .is_some_and(|tip| seq <= *tip)
            {
                self.must_be_persisted.insert(key, seq);
            }
        }
    }

    fn check_fill_precedes_done(&mut self, seq: u64, key: OrderKey, role: &str) {
        self.fills_before_done.checked += 1;
        let Some(order) = self.orders.get(&key) else {
            return;
        };
        if order.phase == Phase::Done {
            let message = format!("FILL at seq {} follows the {}'s ORDER_DONE", seq, role);
            let fields = json!({
                "user_id": key.0,
                "order_id": order_hex(key),
                "role": role,
                "order_done_seq": order.done_seq,
            });
            let violation = self.finding(seq, &message, fields);
            self.fills_before_done.add_violation(violation);
        }
    }

    fn consume_maker(&mut self, seq: u64, record: &FillRecord, key: OrderKey) {
        let Some(order) = self.orders.get(&key) else {
            if self.is_anchored() {
                self.fifo.checked += 1;
                let violation = self.finding(
                    seq,
                    "FILL against a maker that was never inserted",
                    order_fields(key),
                );
                self.fifo.add_violation(violation);
            }
            return;
        };
        self.fifo.checked += 1;
        if order.phase != Phase::Resting {
            let message = format!("FILL against a maker in phase {}", order.phase.as_str());
            let violation = self.finding(seq, &message, order_fields(key));
            self.fifo.add_violation(violation);
            return;
        }
        let (side, price, insert_seq) = (order.side, order.price, order.insert_seq);
        if side == record.taker_side {
            let violation = self.finding(
                seq,
                "FILL between two orders on the same side",
                order_fields(key),
            );
            self.fifo.add_violation(violation);
        }
        if price != record.price.0 {
            let fields = json!({
                "user_id": key.0,
                "order_id": order_hex(key),
                "fill_price": record.price.0,
                "resting_price": price,
            });
            let violation = self.finding(seq, "FILL price differs from the resting price", fields);
            self.fifo.add_violation(violation);
        }
        let front = self
            .levels
            .get(&(side, price))
            .and_then(|level| level.iter().next())
            .map(|(front_seq, front_key)| (*front_seq, *front_key));
        if let Some((front_seq, front_key)) = front {
            if front_seq != insert_seq {
                let fields = json!({
                    "user_id": key.0,
                    "order_id": order_hex(key),
                    "inserted_seq": insert_seq,
                    "price": price,
                    "earlier_order_id": order_hex(front_key),
                    "earlier_user_id": front_key.0,
                    "earlier_inserted_seq": front_seq,
                });
                let violation =
                    self.finding(seq, "FILL skips an earlier order at the same price", fields);
                self.fifo.add_violation(violation);
            }
        }
        let remaining = match self.orders.get_mut(&key) {
            Some(order) => {
                order.remaining -= record.qty.0;
                order.remaining
            }
            None => return,
        };
        if remaining <= 0 {
            self.leave_level(key);
            if let Some(order) = self.orders.get_mut(&key) {
                order.phase = Phase::AwaitingDone;
            }
        }
    }

    fn on_cancelled(&mut self, seq: u64, record: OrderCancelledRecord) {
        let key = (record.user_id, record.order_id_hi, record.order_id_lo);
        let phase = self.orders.get(&key).map(|order| order.phase);
        match phase {
            Some(Phase::Resting) => {
                self.leave_level(key);
                if let Some(order) = self.orders.get_mut(&key) {
                    order.phase = Phase::AwaitingDone;
                }
                self.in_flight = Some(key);
            }
            Some(other) => {
                let message = format!("ORDER_CANCELLED for an order in phase {}", other.as_str());
                self.add_completion_violation(seq, &message, order_fields(key));
            }
            None => {
                if self.is_anchored() {
                    self.add_completion_violation(
                        seq,
                        "ORDER_CANCELLED for an order with no ORDER_ACCEPTED",
                        order_fields(key),
                    );
                }
            }
        }
    }

    fn on_failed(&mut self, seq: u64, record: OrderFailedRecord) {
        if record.reason == FailureReason::DuplicateOrderId as u8 {
            self.duplicates += 1;
            self.in_flight = None;
            return;
        }
        let key = (record.user_id, record.order_id_hi, record.order_id_lo);
        self.complete(seq, key, "ORDER_FAILED", false);
    }

    fn complete(&mut self, seq: u64, key: OrderKey, what: &str, after_leaving_book: bool) {
        let Some(order) = self.orders.get(&key) else {
            if self.is_anchored() {
                let message = format!("{} for an order with no ORDER_ACCEPTED", what);
                self.add_completion_violation(seq, &message, order_fields(key));
            }
            return;
        };
        let phase = order.phase;
        let done_seq = order.done_seq;
        match phase {
            Phase::Done => {
                let message = format!("{} after the order completed at seq {}", what, done_seq);
                self.add_completion_violation(seq, &message, order_fields(key));
                return;
            }
            Phase::AwaitingDone if !after_leaving_book => {
                let message = format!("{} for an order in phase {}", what, phase.as_str());
                self.add_completion_violation(seq, &message, order_fields(key));
                return;
            }
            Phase::Resting => {
                let message = format!("{} while the order still rests", what);
                self.add_completion_violation(seq, &message, order_fields(key));
                self.leave_level(key);
            }
            Phase::Accepted | Phase::AwaitingDone => {}
        }
        if let Some(order) = self.orders.get_mut(&key) {
            order.phase = Phase::Done;
            order.done_seq = seq;
        }
        if self.in_flight == Some(key) {
            self.in_flight = None;
        }
    }

    fn on_bbo(&mut self, seq: u64, record: BboRecord) {
        if record.bid_qty.0 <= 0 || record.ask_qty.0 <= 0 {
            return;
        }
        self.bbo.checked += 1;
        if record.bid_px.0 >= record.ask_px.0 {
            let fields = json!({
                "bid_px": record.bid_px.0,
                "ask_px": record.ask_px.0,
            });
            let violation = self.finding(seq, "BBO bid is not below ask", fields);
            self.bbo.add_violation(violation);
        }
    }

    /// Records that the stream could not be read past its last record.
    pub fn note_read_error(&mut self, error: &str) {
        let message = format!("wal read stopped: {}", error);
        let violation = self.finding(self.last_seq, &message, json!({}));
        self.intact.add_violation(violation);
    }

    /// Reports the orders the stream left without a closing record. Call
    /// once, after the last record. An order cut off by the end of a stream
    /// that is still being written is not reported.
    pub fn finish(&mut self) {
        let is_cut = self.in_flight.is_some();
        let mut stranded: Vec<(u64, Value)> = Vec::new();
        for (key, order) in &self.orders {
            let message = match order.phase {
                Phase::Accepted if self.in_flight != Some(*key) => {
                    "no ORDER_INSERTED or completion followed ORDER_ACCEPTED"
                }
                Phase::AwaitingDone if !is_cut => {
                    "no ORDER_DONE followed the cancel or the last fill"
                }
                _ => continue,
            };
            let seq = order.accepted_seq.max(order.insert_seq);
            stranded.push((seq, self.finding(seq, message, order_fields(*key))));
        }
        stranded.sort_by_key(|(seq, _)| *seq);
        for (_, violation) in stranded {
            self.completion.add_violation(violation);
        }
    }
}
