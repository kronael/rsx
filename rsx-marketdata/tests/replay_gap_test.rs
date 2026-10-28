//! A casting stream loses records until the receiver FAULTs; the gap is
//! served over replication and folded through `apply_record`, so the
//! shadow book ends equal to the book of an ME that saw every record.

use rsx_cast::as_bytes;
use rsx_cast::CastConfig;
use rsx_cast::CastReceiver;
use rsx_cast::CastRecvWith;
use rsx_cast::CastSender;
use rsx_cast::ReplicationService;
use rsx_cast::WalWriter;
use rsx_marketdata::apply::apply_record;
use rsx_marketdata::replay::drain_replay;
use rsx_marketdata::state::MarketDataState;
use rsx_marketdata::types::L2Snapshot;
use rsx_messages::FillRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const STREAM_ID: u32 = 1;
const MAX_OUTBOUND: usize = 16;

fn reserve_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

fn test_tls(dir: &std::path::Path) -> rsx_cast::TlsConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_path = dir.join("repl_cert.pem");
    let key_path = dir.join("repl_key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    rsx_cast::TlsConfig {
        server: Some(rsx_cast::TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: Some(rsx_cast::TlsClient { cert_path }),
    }
}

fn new_state() -> Rc<RefCell<MarketDataState>> {
    let config = SymbolConfig {
        symbol_id: 0,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    };
    Rc::new(RefCell::new(MarketDataState::new(4, config, 256, 100)))
}

fn insert(order_id_lo: u64, price: i64, qty: i64, side: u8) -> OrderInsertedRecord {
    OrderInsertedRecord {
        seq: 0,
        ts_ns: 1000 + order_id_lo,
        symbol_id: 1,
        user_id: 100,
        order_id_hi: 0,
        order_id_lo,
        price: Price(price),
        qty: Qty(qty),
        side,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    }
}

fn cancel(order_id_lo: u64, remaining_qty: i64) -> OrderCancelledRecord {
    OrderCancelledRecord {
        seq: 0,
        ts_ns: 2000,
        symbol_id: 1,
        user_id: 100,
        order_id_hi: 0,
        order_id_lo,
        remaining_qty: Qty(remaining_qty),
        reason: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        _pad2: [0; 8],
    }
}

fn fill(maker_order_id_lo: u64, price: i64, qty: i64) -> FillRecord {
    FillRecord {
        seq: 0,
        ts_ns: 3000,
        symbol_id: 1,
        taker_user_id: 102,
        maker_user_id: 100,
        taker_fee_bps: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 9,
        maker_order_id_hi: 0,
        maker_order_id_lo,
        price: Price(price),
        qty: Qty(qty),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        maker_fee_bps: 0,
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

fn snapshot(state: &Rc<RefCell<MarketDataState>>) -> L2Snapshot {
    let mut state = state.borrow_mut();
    state.book_mut(1).expect("book").derive_l2_snapshot(10)
}

#[test]
fn faulted_gap_replay_converges_shadow_book() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(faulted_gap_replay_converges)
        .unwrap()
        .join()
        .unwrap();
}

fn faulted_gap_replay_converges() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    let sender_wal_dir = tmp.path().join("sender_wal");
    std::fs::create_dir_all(&wal_dir).unwrap();
    std::fs::create_dir_all(&sender_wal_dir).unwrap();
    let tls = test_tls(tmp.path());

    let mut insert_bid = insert(1, 100, 10, 0);
    let mut insert_ask = insert(2, 101, 20, 1);
    let mut insert_lost = insert(3, 99, 7, 0);
    let mut cancel_lost = cancel(1, 10);
    let mut fill_live = fill(2, 101, 5);

    let mut writer = WalWriter::new(STREAM_ID, &wal_dir, 64 * 1024 * 1024).unwrap();
    let framed_bid = writer.prepare(&mut insert_bid).unwrap();
    writer.append_framed(&framed_bid).unwrap();
    let framed_ask = writer.prepare(&mut insert_ask).unwrap();
    writer.append_framed(&framed_ask).unwrap();
    let framed_lost = writer.prepare(&mut insert_lost).unwrap();
    writer.append_framed(&framed_lost).unwrap();
    let framed_cancel = writer.prepare(&mut cancel_lost).unwrap();
    writer.append_framed(&framed_cancel).unwrap();
    writer.flush().unwrap();

    let replay_addr = reserve_port();
    let service = ReplicationService::new(wal_dir, tls.clone()).unwrap();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move { service.serve(replay_addr).await.unwrap() });
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::net::TcpStream::connect(replay_addr).is_err() {
        assert!(Instant::now() < deadline, "replication server never bound");
        std::thread::sleep(Duration::from_millis(20));
    }

    let recv_addr = {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.local_addr().unwrap()
    };
    let config = CastConfig {
        heartbeat_interval_ms: 2,
        max_nak_retries: 2,
        nak_debounce_us: 1000,
        ..CastConfig::default()
    };
    let mut sender =
        CastSender::with_config(recv_addr, STREAM_ID, &sender_wal_dir, &config).unwrap();
    let mut receiver =
        CastReceiver::with_config(recv_addr, sender.local_addr().unwrap(), &config).unwrap();

    sender.send_framed(&framed_bid).unwrap();
    sender.send_framed(&framed_ask).unwrap();
    sender.send_framed(&framed_cancel).unwrap();

    let shadow = new_state();
    let deadline = Instant::now() + Duration::from_secs(5);
    let last_delivered_seq = loop {
        assert!(Instant::now() < deadline, "receiver never FAULTed");
        sender.tick().unwrap();
        let recv = receiver.try_recv_with(|hdr, payload| {
            apply_record(&shadow, hdr.record_type, payload, MAX_OUTBOUND);
        });
        if let CastRecvWith::Faulted {
            last_delivered_seq, ..
        } = recv
        {
            break last_delivered_seq;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(last_delivered_seq, 2);
    assert!(shadow.borrow_mut().book_mut(1).is_some());

    let new_tip = drain_replay(
        STREAM_ID,
        replay_addr.to_string(),
        last_delivered_seq,
        tmp.path().join("tip"),
        tls,
        |raw| apply_record(&shadow, raw.header.record_type, &raw.payload, MAX_OUTBOUND),
    )
    .unwrap();
    assert_eq!(new_tip, 4);
    receiver.reset_after_replay(new_tip);

    let framed_fill = writer.prepare(&mut fill_live).unwrap();
    sender.send_framed(&framed_fill).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "live record never arrived");
        let recv = receiver.try_recv_with(|hdr, payload| {
            apply_record(&shadow, hdr.record_type, payload, MAX_OUTBOUND);
        });
        if matches!(recv, CastRecvWith::Data) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    let me_book = new_state();
    let log = [
        (RECORD_ORDER_INSERTED, as_bytes(&insert_bid)),
        (RECORD_ORDER_INSERTED, as_bytes(&insert_ask)),
        (RECORD_ORDER_INSERTED, as_bytes(&insert_lost)),
        (RECORD_ORDER_CANCELLED, as_bytes(&cancel_lost)),
        (RECORD_FILL, as_bytes(&fill_live)),
    ];
    for (record_type, payload) in log {
        apply_record(&me_book, record_type, payload, MAX_OUTBOUND);
    }

    let shadow_l2 = snapshot(&shadow);
    let me_l2 = snapshot(&me_book);
    assert_eq!(shadow_l2.bids, me_l2.bids);
    assert_eq!(shadow_l2.asks, me_l2.asks);
    assert_eq!(shadow_l2.bids.len(), 1);
    assert_eq!(shadow_l2.bids[0].price, 99);
    assert_eq!(shadow_l2.bids[0].qty, 7);
    assert_eq!(shadow_l2.asks[0].qty, 15);
}
