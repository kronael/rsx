//! CMP v4 FAULTED → DXS replay POC end-to-end test.
//!
//! Simulates: risk process has WAL-persisted OrderRequest
//! records that matching never saw (some were dropped on
//! the UDP wire — far enough to escape NAK and trip
//! FAULTED). The matching tile opens a ReplicationConsumer against
//! risk's DXS server, drains Phase 1 until CAUGHT_UP, and
//! applies each record. We assert the resulting book state
//! matches what a live session of the same records would
//! produce.

use rsx_book::book::Orderbook;
use rsx_book::matching::IncomingOrder;
use rsx_book::matching::process_new_order;
use rsx_cast::ReplicationService;
use rsx_cast::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::replay::drain_dxs_replay_into_book;
use rsx_matching::wal_integration::OrderKey;
use rsx_matching::wire::OrderMessage;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rustc_hash::FxHashMap;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

const SYM: u32 = 1;

fn cfg() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYM,
        price_decimals: 2,
        qty_decimals: 3,
        tick_size: 1,
        lot_size: 1,
    }
}

fn reserve_port() -> SocketAddr {
    let listener =
        TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

/// Build an OrderMessage as it would arrive from risk on
/// the CMP wire. WalWriter will assign seq via set_seq.
fn order_msg(
    uid: u32,
    oid: u64,
    side: u8,
    px: i64,
    qty: i64,
) -> OrderMessage {
    OrderMessage {
        seq: 0,
        price: px,
        qty,
        side,
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        _pad1: [0; 4],
        user_id: uid,
        _pad2: 0,
        timestamp_ns: 1_000,
        order_id_hi: 0,
        order_id_lo: oid,
    }
}

/// Populate a "risk" WAL with N order requests. Returns the
/// last seq written. The DXS server stands these up over
/// TCP via [`ReplicationService`].
fn write_risk_wal(
    wal_dir: &Path,
    stream_id: u32,
) -> u64 {
    let mut writer = WalWriter::new(
        stream_id, wal_dir, 64 * 1024 * 1024,
    )
    .unwrap();
    // The seq is assigned by WalWriter::append via set_seq
    // (CastRecord trait). OrderMessage isn't a CastRecord, so
    // we wrap each as a raw record. Easier: just append the
    // OrderMessage bytes via the writer's raw API... but the
    // simplest path that exists is to manually call append
    // on a struct that implements CastRecord with type
    // RECORD_ORDER_REQUEST. OrderMessage doesn't impl
    // CastRecord; let's use a local newtype wrapper.
    //
    // Easier still: we know seq is the first 8 bytes per
    // CastRecord convention. WalWriter::append_raw_payload
    // doesn't exist — but the OrderMessage layout starts
    // with `pub seq: u64`. We can write via a thin shim
    // type that implements CastRecord.
    for (uid, oid, side, px, qty) in [
        (10u32, 1u64, 0u8, 100i64, 5i64),  // buy 5 @ 100
        (10u32, 2u64, 0u8, 101i64, 3i64),  // buy 3 @ 101
        (20u32, 3u64, 1u8, 105i64, 4i64),  // sell 4 @ 105
        (20u32, 4u64, 1u8, 110i64, 2i64),  // sell 2 @ 110
        (30u32, 5u64, 0u8, 107i64, 6i64),  // buy 6 @ 107 — matches 4@105 + 2@?... no asks left at 107
    ] {
        let mut wrapped =
            OrderMessageWire(order_msg(uid, oid, side, px, qty));
        {
            let framed = writer.prepare(&mut wrapped).unwrap();
            writer.append_framed(&framed).unwrap();
        }
    }
    writer.flush().unwrap();
    writer.last_seq()
}

/// CastRecord shim so WalWriter::append can write
/// OrderMessage bytes with the right record_type. We keep
/// this in the test (not main) so the prod path remains
/// the canonical wire shape.
#[repr(C)]
#[derive(Copy, Clone)]
struct OrderMessageWire(OrderMessage);

impl rsx_cast::records::CastRecord for OrderMessageWire {
    fn seq(&self) -> u64 {
        self.0.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.0.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_REQUEST
    }
}

/// Build the expected book by running the same orders
/// live through process_new_order. The replay path must
/// produce an equivalent book.
fn expected_book() -> Orderbook {
    let mut book = Orderbook::new(cfg(), 65_536, 50_000);
    for (uid, oid, side, px, qty) in [
        (10u32, 1u64, Side::Buy, 100i64, 5i64),
        (10u32, 2u64, Side::Buy, 101i64, 3i64),
        (20u32, 3u64, Side::Sell, 105i64, 4i64),
        (20u32, 4u64, Side::Sell, 110i64, 2i64),
        (30u32, 5u64, Side::Buy, 107i64, 6i64),
    ] {
        let mut incoming = IncomingOrder {
            price: px,
            qty,
            remaining_qty: qty,
            side,
            tif: TimeInForce::GTC,
            user_id: uid,
            reduce_only: false,
            post_only: false,
            timestamp_ns: 1_000,
            order_id_hi: 0,
            order_id_lo: oid,
        };
        process_new_order(&mut book, &mut incoming);
    }
    book
}

/// Capture observable resting-order state for equality.
/// Mirrors the comparison used in replay_after_snapshot_test.
fn book_state(book: &Orderbook) -> Vec<(i64, i64, u8)> {
    let mut out: Vec<(i64, i64, u8)> = Vec::new();
    for i in 0..book.orders.len() {
        let slot = book.orders.get(i);
        if slot.is_active() {
            out.push((
                slot.price.0,
                slot.remaining_qty.0,
                slot.side,
            ));
        }
    }
    out.sort();
    out
}

#[test]
fn faulted_recovers_via_dxs_replay() {
    let tmp = TempDir::new().unwrap();
    let risk_wal_dir = tmp.path().join("risk_wal");
    std::fs::create_dir_all(&risk_wal_dir).unwrap();
    let me_wal_dir = tmp.path().join("me_wal");
    std::fs::create_dir_all(&me_wal_dir).unwrap();

    let stream_id = SYM;
    let last_seq = write_risk_wal(&risk_wal_dir, stream_id);
    assert!(last_seq >= 5);

    let replay_addr = reserve_port();

    // Stand up risk's DXS server in a background thread on
    // a dedicated tokio runtime — same shape as the
    // matching binary's DXS sidecar.
    let risk_wal_for_server = risk_wal_dir.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder
            ::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let service = ReplicationService::new(
            risk_wal_for_server, None,
        )
        .unwrap();
        rt.block_on(async move {
            service.serve(replay_addr).await.unwrap();
        });
    });
    // Wait for the server to bind. Loop-poll keeps the test
    // robust under load.
    let deadline = std::time::Instant::now()
        + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err()
    {
        if std::time::Instant::now() > deadline {
            panic!("DXS server failed to bind");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    // ME-side: book starts empty, no records seen yet
    // (simulates FAULTED on first packet). last_delivered_seq
    // = 0 → DXS replay covers the whole stream.
    let mut book = Orderbook::new(cfg(), 65_536, 50_000);
    let mut order_index: FxHashMap<OrderKey, u32> =
        FxHashMap::default();
    let mut dedup = DedupTracker::new();
    let mut me_writer = WalWriter::new(
        SYM, &me_wal_dir, 64 * 1024 * 1024,
    )
    .unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let tip_file = tmp.path().join("me_replay_tip.bin");

    let new_tip = drain_dxs_replay_into_book(
        &rt,
        &mut book,
        &mut order_index,
        &mut dedup,
        &mut me_writer,
        SYM,
        replay_addr.to_string(),
        0, // last_delivered_seq
        tip_file,
    )
    .expect("drain failed");

    // We expect all five OrderRequests to have been applied.
    assert_eq!(new_tip, last_seq);
    let expected = expected_book();
    assert_eq!(
        book_state(&book),
        book_state(&expected),
        "replay-built book differs from live-built book",
    );
}
