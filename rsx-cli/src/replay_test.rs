use crate::fixture_utils::Wal;
use crate::fixture_utils::BUY;
use crate::fixture_utils::SELL;
use crate::fixture_utils::SYMBOL;
use crate::ledger::Ledger;
use crate::replay::judge;
use crate::replay::recover_book;
use crate::verify::scan_stream;
use rsx_book::book::Orderbook;
use rsx_book::event::Event;
use rsx_book::event::OrderCancelReason;
use rsx_book::event::OrderDoneReason;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::wal::WalWriter;
use rsx_matching::config::BootConfig;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::recovery::Recovered;
use rsx_matching::wal::write_events_to_wal;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rustc_hash::FxHashMap;
use std::path::Path;
use tempfile::TempDir;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYMBOL,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn boot() -> BootConfig {
    BootConfig {
        book_capacity: 1024,
        ..BootConfig::env(config())
    }
}

fn book() -> Orderbook {
    Orderbook::new(config(), 1024, 50_000)
}

fn rest(book: &mut Orderbook, user_id: u32, order_id: u64, side: Side, price: i64, qty: i64) {
    book.insert_resting(price, qty, side, 0, user_id, false, 1, 0, order_id);
}

fn recovered(book: Orderbook, checkpoint_wal_seq: u64) -> Recovered {
    Recovered {
        book,
        order_index: FxHashMap::default(),
        dedup: DedupTracker::new(),
        next_seq: 100,
        checkpoint_wal_seq,
    }
}

fn ledger_of(wal: &Wal) -> Ledger<'static> {
    let mut ledger = Ledger::new(Path::new("wal"), SYMBOL, None);
    for (record_type, payload) in &wal.records {
        ledger.apply(*record_type, payload);
    }
    ledger.finish();
    ledger
}

fn resting_buy() -> Wal {
    let mut wal = Wal::default();
    wal.accepted(1, 1, BUY, 100, 5).inserted(1, 1, BUY, 100, 5);
    wal
}

#[test]
fn a_book_that_holds_what_the_wal_says_rests_matches() {
    let mut replayed_book = book();
    rest(&mut replayed_book, 1, 1, Side::Buy, 100, 5);
    let (replayed, skip) = judge(&ledger_of(&resting_buy()), &recovered(replayed_book, 0));
    assert_eq!(skip, None);
    assert_eq!(replayed.matches.checked, 1);
    assert_eq!(replayed.matches.violation_count, 0);
    assert_eq!(replayed.crossed.violation_count, 0);
    assert_eq!(replayed.slab.violation_count, 0);
}

#[test]
fn a_book_with_more_remaining_than_the_wal_records_differs() {
    let mut wal = resting_buy();
    wal.accepted(2, 2, SELL, 100, 3)
        .fill((2, 2), (1, 1), 100, 3, SELL)
        .done(2, 2);
    let mut replayed_book = book();
    rest(&mut replayed_book, 1, 1, Side::Buy, 100, 5);
    let (replayed, _) = judge(&ledger_of(&wal), &recovered(replayed_book, 0));
    assert_eq!(replayed.matches.violation_count, 1);
    let violation = &replayed.matches.violations[0];
    assert_eq!(violation["wal"]["remaining"], 2);
    assert_eq!(violation["book"]["remaining"], 5);
}

#[test]
fn an_order_resting_in_the_wal_but_missing_from_the_book_is_flagged() {
    let (replayed, _) = judge(&ledger_of(&resting_buy()), &recovered(book(), 0));
    assert_eq!(replayed.matches.violation_count, 1);
    assert_eq!(replayed.matches.violations[0]["user_id"], 1);
}

#[test]
fn an_order_resting_in_the_book_but_not_in_the_wal_is_flagged() {
    let mut replayed_book = book();
    rest(&mut replayed_book, 1, 1, Side::Buy, 100, 5);
    rest(&mut replayed_book, 9, 9, Side::Sell, 120, 5);
    let (replayed, _) = judge(&ledger_of(&resting_buy()), &recovered(replayed_book, 0));
    assert_eq!(replayed.matches.violation_count, 1);
    assert_eq!(replayed.matches.violations[0]["user_id"], 9);
}

#[test]
fn a_book_with_a_buy_at_or_above_a_sell_is_crossed() {
    let mut replayed_book = book();
    rest(&mut replayed_book, 1, 1, Side::Buy, 101, 5);
    rest(&mut replayed_book, 2, 2, Side::Sell, 100, 5);
    let (replayed, _) = judge(&ledger_of(&Wal::default()), &recovered(replayed_book, 0));
    assert_eq!(replayed.crossed.violation_count, 1);
    assert_eq!(replayed.crossed.violations[0]["best_bid"], 101);
    assert_eq!(replayed.crossed.violations[0]["best_ask"], 100);
}

#[test]
fn a_book_with_a_buy_equal_to_a_sell_is_crossed() {
    let mut replayed_book = book();
    rest(&mut replayed_book, 1, 1, Side::Buy, 100, 5);
    rest(&mut replayed_book, 2, 2, Side::Sell, 100, 5);
    let (replayed, _) = judge(&ledger_of(&Wal::default()), &recovered(replayed_book, 0));
    assert_eq!(replayed.crossed.violation_count, 1);
}

#[test]
fn a_book_with_a_gap_between_bid_and_ask_is_not_crossed() {
    let mut replayed_book = book();
    rest(&mut replayed_book, 1, 1, Side::Buy, 99, 5);
    rest(&mut replayed_book, 2, 2, Side::Sell, 100, 5);
    let (replayed, _) = judge(&ledger_of(&Wal::default()), &recovered(replayed_book, 0));
    assert_eq!(replayed.crossed.checked, 1);
    assert_eq!(replayed.crossed.violation_count, 0);
}

#[test]
fn a_slot_allocated_but_neither_free_nor_active_is_a_leak() {
    let mut replayed_book = book();
    replayed_book.orders.alloc();
    let (replayed, _) = judge(&ledger_of(&Wal::default()), &recovered(replayed_book, 0));
    assert_eq!(replayed.slab.violation_count, 1);
    assert_eq!(replayed.slab.violations[0]["allocated"], 1);
    assert_eq!(replayed.slab.violations[0]["active"], 0);
}

#[test]
fn a_freed_slot_is_not_a_leak() {
    let mut replayed_book = book();
    let handle = replayed_book.orders.alloc();
    replayed_book.orders.free(handle);
    let (replayed, _) = judge(&ledger_of(&Wal::default()), &recovered(replayed_book, 0));
    assert_eq!(replayed.slab.checked, 1);
    assert_eq!(replayed.slab.violation_count, 0);
}

#[test]
fn a_stream_cut_inside_a_match_cycle_is_not_compared() {
    let mut wal = resting_buy();
    wal.accepted(2, 2, SELL, 100, 3);
    let (replayed, skip) = judge(&ledger_of(&wal), &recovered(book(), 0));
    assert!(skip.is_some_and(|reason| reason.contains("match cycle")));
    assert_eq!(replayed.matches.checked, 0);
    assert_eq!(replayed.slab.checked, 1);
}

#[test]
fn a_pruned_stream_without_a_checkpoint_is_not_compared() {
    let mut wal = Wal::starting_at(7);
    wal.accepted(1, 1, BUY, 100, 5).inserted(1, 1, BUY, 100, 5);
    let (replayed, skip) = judge(&ledger_of(&wal), &recovered(book(), 0));
    assert!(skip.is_some_and(|reason| reason.contains("no checkpoint reaches it")));
    assert_eq!(replayed.matches.checked, 0);
}

#[test]
fn a_pruned_stream_is_compared_when_a_checkpoint_reaches_it() {
    let mut wal = Wal::starting_at(7);
    wal.accepted(1, 1, BUY, 100, 5).inserted(1, 1, BUY, 100, 5);
    let mut replayed_book = book();
    rest(&mut replayed_book, 1, 1, Side::Buy, 100, 5);
    rest(&mut replayed_book, 5, 5, Side::Sell, 130, 5);
    let (replayed, skip) = judge(&ledger_of(&wal), &recovered(replayed_book, 6));
    assert_eq!(skip, None);
    assert_eq!(replayed.matches.checked, 1);
    assert_eq!(replayed.matches.violation_count, 0);
}

#[test]
fn a_checkpoint_ahead_of_the_wal_is_flagged() {
    let (replayed, _) = judge(&ledger_of(&resting_buy()), &recovered(book(), 50));
    assert_eq!(replayed.matches.violation_count, 1);
    assert_eq!(replayed.matches.violations[0]["checkpoint_wal_seq"], 50);
}

struct Engine {
    book: Orderbook,
    writer: WalWriter,
}

impl Engine {
    fn new(dir: &TempDir) -> Self {
        Engine {
            book: book(),
            writer: WalWriter::new(SYMBOL, dir.path(), 64 * 1024 * 1024).expect("open wal"),
        }
    }

    fn append<T: rsx_cast::CastRecord>(&mut self, record: &mut T) {
        let framed = self.writer.prepare(record).expect("frame record");
        self.writer.append_framed(&framed).expect("append record");
    }

    fn place(&mut self, user_id: u32, order_id: u64, side: Side, price: i64, qty: i64) {
        self.place_dropping(user_id, order_id, side, price, qty, |_| false);
    }

    fn place_dropping(
        &mut self,
        user_id: u32,
        order_id: u64,
        side: Side,
        price: i64,
        qty: i64,
        is_dropped: fn(&Event) -> bool,
    ) {
        let mut accepted = OrderAcceptedRecord {
            seq: 0,
            ts_ns: 1,
            user_id,
            symbol_id: SYMBOL,
            order_id_hi: 0,
            order_id_lo: order_id,
            price,
            qty,
            side: side as u8,
            tif: 0,
            reduce_only: 0,
            post_only: 0,
            cid: [0; 20],
            _pad: [0; 48],
        };
        self.append(&mut accepted);
        let mut incoming = IncomingOrder {
            price,
            qty,
            remaining_qty: qty,
            side,
            tif: TimeInForce::GTC,
            user_id,
            reduce_only: false,
            post_only: false,
            timestamp_ns: 1,
            order_id_hi: 0,
            order_id_lo: order_id,
        };
        process_new_order(&mut self.book, &mut incoming);
        self.book.event_buf.retain(|event| !is_dropped(event));
        write_events_to_wal(&mut self.writer, &self.book, SYMBOL, 1).expect("write events");
    }

    fn cancel(&mut self, user_id: u32, order_id: u64, handle: u32, remaining: i64) {
        self.book.cancel_order(handle);
        self.book.event_buf.clear();
        self.book.emit(Event::OrderCancelled {
            handle,
            user_id,
            remaining_qty: Qty(remaining),
            reason: OrderCancelReason::User,
            order_id_hi: 0,
            order_id_lo: order_id,
        });
        self.book.emit(Event::OrderDone {
            handle,
            tif: 0,
            reduce_only: false,
            user_id,
            reason: OrderDoneReason::Cancelled,
            filled_qty: Qty(0),
            remaining_qty: Qty(remaining),
            order_id_hi: 0,
            order_id_lo: order_id,
        });
        write_events_to_wal(&mut self.writer, &self.book, SYMBOL, 1).expect("write events");
    }

    fn finish(mut self) {
        self.writer.flush().expect("flush wal");
    }
}

fn replay_of(dir: &TempDir) -> (Ledger<'static>, Recovered) {
    let recovered = recover_book(boot(), dir.path()).expect("replay");
    let ledger = scan_stream(dir.path(), SYMBOL, None, None).expect("scan");
    (ledger, recovered)
}

#[test]
fn a_wal_the_engine_wrote_replays_to_the_book_it_records() {
    let dir = TempDir::new().expect("tempdir");
    let mut engine = Engine::new(&dir);
    engine.place(1, 1, Side::Buy, 100, 5);
    engine.place(2, 2, Side::Buy, 100, 5);
    engine.place(3, 3, Side::Buy, 99, 4);
    engine.place(4, 4, Side::Sell, 100, 7);
    engine.place(5, 5, Side::Sell, 105, 2);
    engine.cancel(3, 3, 2, 4);
    engine.finish();

    let (ledger, recovered) = replay_of(&dir);
    let (replayed, skip) = judge(&ledger, &recovered);
    assert_eq!(skip, None);
    assert_eq!(ledger.completion.violation_count, 0);
    assert_eq!(ledger.fifo.violation_count, 0);
    assert_eq!(replayed.matches.violation_count, 0);
    assert_eq!(replayed.crossed.violation_count, 0);
    assert_eq!(replayed.slab.violation_count, 0);
    assert_eq!(ledger.open_orders().count(), 2);
}

#[test]
fn a_wal_missing_an_order_inserted_record_differs_from_its_replay() {
    let dir = TempDir::new().expect("tempdir");
    let mut engine = Engine::new(&dir);
    engine.place_dropping(1, 1, Side::Buy, 100, 5, |event| {
        matches!(event, Event::OrderInserted { .. })
    });
    engine.place(2, 2, Side::Buy, 99, 5);
    engine.finish();

    let (ledger, recovered) = replay_of(&dir);
    let (replayed, _) = judge(&ledger, &recovered);
    assert_eq!(ledger.completion.violation_count, 1);
    assert_eq!(replayed.matches.violation_count, 1);
    assert_eq!(
        replayed.matches.violations[0]["message"],
        "rests in the replayed book, not in the WAL"
    );
    assert_eq!(replayed.matches.violations[0]["user_id"], 1);
}

#[test]
fn a_wal_record_too_short_to_replay_is_an_error() {
    let dir = TempDir::new().expect("tempdir");
    let mut wal = Wal::default();
    wal.raw(RECORD_ORDER_ACCEPTED, 16);
    wal.write_stream(dir.path(), SYMBOL);
    let error = recover_book(boot(), dir.path())
        .err()
        .expect("replay error");
    assert!(error.contains("16-byte payload"), "{}", error);
}
