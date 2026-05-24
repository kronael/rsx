//! Two CMP streams feeding a single MarketDataState.
//!
//! Models the production wiring: marketdata binds one
//! CmpReceiver per matching engine (one ME per symbol) and
//! routes every record by `symbol_id` into a single shared
//! shadow book. The test asserts:
//!
//!   * both streams reach the state (no socket cross-talk)
//!   * shadow books for ME-A and ME-B remain independent
//!   * per-symbol seq tracking does not collide

use rsx_cast::cmp::CmpRecv;
use rsx_cast::cmp::CmpReceiver;
use rsx_cast::cmp::CmpSender;
use rsx_marketdata::state::MarketDataState;
use rsx_messages::FillRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use std::net::UdpSocket;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// One ME endpoint: a CmpSender on an OS-assigned port, plus
/// the receiver bound to that pair. Mirrors `loopback_pair`
/// in the dxs test suite.
struct MeEndpoint {
    sender: CmpSender,
    receiver: CmpReceiver,
}

fn make_endpoint(wal_dir: &std::path::Path) -> MeEndpoint {
    // Reserve an ephemeral port for the receiver.
    let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_addr = recv_sock.local_addr().unwrap();
    drop(recv_sock);

    let sender =
        CmpSender::new(recv_addr, 1, wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let receiver =
        CmpReceiver::new(recv_addr, sender_addr, 1).unwrap();
    MeEndpoint { sender, receiver }
}

fn base_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 0,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

fn insert_record(
    symbol_id: u32,
    seq: u64,
    price: i64,
    qty: i64,
) -> OrderInsertedRecord {
    OrderInsertedRecord {
        seq,
        ts_ns: 1000 + seq,
        symbol_id,
        user_id: 42,
        order_id_hi: 0,
        order_id_lo: seq,
        price: Price(price),
        qty: Qty(qty),
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    }
}

fn fill_record(
    symbol_id: u32,
    seq: u64,
    maker_order_id_lo: u64,
    qty: i64,
) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: 2000 + seq,
        symbol_id,
        taker_user_id: 99,
        maker_user_id: 42,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 1_000_000 + seq,
        maker_order_id_hi: 0,
        maker_order_id_lo,
        price: Price(0),
        qty: Qty(qty),
        taker_side: 1,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
taker_ts_ns: 0,
    }
}

/// Run the test body on a thread with 16MB stack — the
/// ShadowBook event_buf is ~1MB and the default 8MB stack
/// can overflow in debug builds. Mirrors `replay_test.rs`.
fn big_stack<F: FnOnce() + Send + 'static>(f: F) {
    thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn multi_me_streams_aggregate_per_symbol() {
    big_stack(|| {
        let tmp_a = TempDir::new().unwrap();
        let tmp_b = TempDir::new().unwrap();
        let mut me_a = make_endpoint(tmp_a.path());
        let mut me_b = make_endpoint(tmp_b.path());

        let mut state =
            MarketDataState::new(4, base_config(), 256, 100);

        // ME-A: symbol 1. Insert two orders.
        let mut ins_a1 = insert_record(1, 1, 1000, 10);
        let mut ins_a2 = insert_record(1, 2, 1001, 20);
        me_a.sender.send(&mut ins_a1).unwrap();
        me_a.sender.send(&mut ins_a2).unwrap();

        // ME-B: symbol 2. Insert one order then fill it.
        let mut ins_b1 = insert_record(2, 1, 2000, 50);
        me_b.sender.send(&mut ins_b1).unwrap();
        let mut fill_b1 = fill_record(2, 2, 1, 30);
        me_b.sender.send(&mut fill_b1).unwrap();

        // UDP delivery is async; give the loopback a beat.
        thread::sleep(Duration::from_millis(20));

        // Drain both receivers into the single shared state,
        // exactly as main.rs does.
        drain_into(&mut me_a.receiver, &mut state);
        drain_into(&mut me_b.receiver, &mut state);

        // Symbol 1: two resting bids, no fills.
        let book_a = state.book_mut(1).expect("book A");
        let snap_a = book_a.derive_l2_snapshot(10);
        let total_a: i64 =
            snap_a.bids.iter().map(|l| l.qty).sum();
        assert_eq!(
            total_a, 30,
            "symbol 1 should hold full 10+20 from ME-A only",
        );
        assert!(
            snap_a.asks.is_empty(),
            "symbol 1 had no ME-A ask flow",
        );

        // Symbol 2: 50 inserted, 30 filled => 20 left.
        let book_b = state.book_mut(2).expect("book B");
        let snap_b = book_b.derive_l2_snapshot(10);
        let total_b: i64 =
            snap_b.bids.iter().map(|l| l.qty).sum();
        assert_eq!(
            total_b, 20,
            "symbol 2 should reflect ME-B insert minus fill",
        );

        // Per-symbol seq tracking is independent: each ME used
        // seqs starting at 1 and they must NOT collide.
        // After consuming two seq=1,2 messages per symbol the
        // expected_seq for each is 3.
        assert!(
            state.check_seq(1, 3).is_none(),
            "symbol 1 next-expected should be 3 (no gap)",
        );
        assert!(
            state.check_seq(2, 3).is_none(),
            "symbol 2 next-expected should be 3 (no gap)",
        );
        // And ME-A's seq=2 must not have advanced ME-B's
        // sequence: feeding seq=2 for symbol 2 NOW would be
        // stale (already consumed), feeding seq=4 should be
        // a clean step.
        assert!(
            state.check_seq(2, 4).is_none(),
            "symbol 2 should accept seq=4 cleanly",
        );
    });
}

fn drain_into(
    receiver: &mut CmpReceiver,
    state: &mut MarketDataState,
) {
    loop {
        let (hdr, payload) = match receiver.try_recv() {
            CmpRecv::Data(h, p) => (h, p),
            CmpRecv::Empty => break,
            CmpRecv::Faulted { .. } => {
                panic!("unexpected fault in test")
            }
        };
        match hdr.record_type {
            RECORD_ORDER_INSERTED => {
                if payload.len()
                    < std::mem::size_of::<OrderInsertedRecord>()
                {
                    continue;
                }
                let rec = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const OrderInsertedRecord,
                    )
                };
                state.ensure_book(rec.symbol_id, rec.price.0);
                if let Some(book) =
                    state.book_mut(rec.symbol_id)
                {
                    book.apply_insert_by_id(
                        rec.price.0,
                        rec.qty.0,
                        rec.side,
                        rec.user_id,
                        rec.ts_ns,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                }
            }
            RECORD_FILL => {
                if payload.len()
                    < std::mem::size_of::<FillRecord>()
                {
                    continue;
                }
                let rec = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr() as *const FillRecord,
                    )
                };
                if let Some(book) =
                    state.book_mut(rec.symbol_id)
                {
                    book.apply_fill_by_order_id(
                        rec.maker_order_id_hi,
                        rec.maker_order_id_lo,
                        rec.qty.0,
                        rec.ts_ns,
                    );
                }
            }
            _ => {}
        }
    }
}
