use rsx_dxs::DxsReplayService;
use rsx_dxs::FillRecord;
use rsx_dxs::OrderInsertedRecord;
use rsx_dxs::WalWriter;
use rsx_marketdata::replay::run_replay_bootstrap_blocking;
use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;
use std::net::SocketAddr;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn dxs_replay_rebuilds_shadow_book() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let stream_id = 1u32;
    let mut writer = WalWriter::new(
        stream_id,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    let mut insert1 = OrderInsertedRecord {
        seq: 0,
        ts_ns: 1000,
        symbol_id: 1,
        user_id: 100,
        order_id_hi: 0,
        order_id_lo: 1,
        price: 100,
        qty: 10,
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };

    let mut insert2 = OrderInsertedRecord {
        seq: 0,
        ts_ns: 2000,
        symbol_id: 1,
        user_id: 101,
        order_id_hi: 0,
        order_id_lo: 2,
        price: 101,
        qty: 20,
        side: 1,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };

    let mut fill = FillRecord {
        seq: 0,
        ts_ns: 3000,
        symbol_id: 1,
        taker_user_id: 102,
        maker_user_id: 100,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 3,
        maker_order_id_hi: 0,
        maker_order_id_lo: 1,
        price: 100,
        qty: 5,
        taker_side: 1,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };

    writer.append(&mut insert1).unwrap();
    writer.append(&mut insert2).unwrap();
    writer.append(&mut fill).unwrap();
    writer.flush().unwrap();

    let replay_addr: SocketAddr = "127.0.0.1:19200"
        .parse()
        .unwrap();
    let server =
        DxsReplayService::new(wal_dir, None).unwrap();

    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            server.serve(replay_addr).await
        })
    });
    thread::sleep(Duration::from_millis(100));

    let tip_file = tmp.path().join("tip");
    let result = run_replay_bootstrap_blocking(
        stream_id,
        replay_addr.to_string(),
        tip_file,
    )
    .unwrap();

    assert_eq!(result.events.len(), 3);
    assert!(result.caught_up);
    assert_eq!(result.last_seq, 3);

    let base_config = SymbolConfig {
        symbol_id: 0,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(
        64,
        base_config,
        256,
        100,
    );

    for event in result.events {
        if let Some(rec) = event.insert {
            state.ensure_book(rec.symbol_id, rec.price);
            if let Some(book) = state.book_mut(rec.symbol_id)
            {
                book.apply_insert_by_id(
                    rec.price,
                    rec.qty,
                    rec.side,
                    rec.user_id,
                    rec.ts_ns,
                    rec.order_id_hi,
                    rec.order_id_lo,
                );
            }
        } else if let Some(rec) = event.fill {
            if let Some(book) = state.book_mut(rec.symbol_id)
            {
                book.apply_fill_by_order_id(
                    rec.maker_order_id_hi,
                    rec.maker_order_id_lo,
                    rec.qty,
                    rec.ts_ns,
                );
            }
        }
    }

    if let Some(book) = state.book_mut(1) {
        let bbo = book.derive_bbo();
        assert!(bbo.is_some());
        let bbo = bbo.unwrap();
        assert_eq!(bbo.bid_px, 100);
        assert_eq!(bbo.bid_qty, 5);
        assert_eq!(bbo.ask_px, 101);
        assert_eq!(bbo.ask_qty, 20);
    } else {
        panic!("book not found");
    }
}

#[test]
fn recovery_from_me_wal_then_live() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let stream_id = 1u32;
    let mut writer = WalWriter::new(
        stream_id,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    let mut insert = OrderInsertedRecord {
        seq: 0,
        ts_ns: 1000,
        symbol_id: 1,
        user_id: 100,
        order_id_hi: 0,
        order_id_lo: 1,
        price: 100,
        qty: 10,
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };

    writer.append(&mut insert).unwrap();
    writer.flush().unwrap();

    let replay_addr: SocketAddr = "127.0.0.1:19201"
        .parse()
        .unwrap();
    let server =
        DxsReplayService::new(wal_dir, None).unwrap();

    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            server.serve(replay_addr).await
        })
    });
    thread::sleep(Duration::from_millis(100));

    let tip_file = tmp.path().join("tip");
    let result = run_replay_bootstrap_blocking(
        stream_id,
        replay_addr.to_string(),
        tip_file,
    )
    .unwrap();

    assert!(result.caught_up);
    assert_eq!(result.last_seq, 1);
}

#[test]
fn recovery_snapshot_sent_after_catchup() {
    let tmp = TempDir::new().unwrap();
    let wal_dir = tmp.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();

    let stream_id = 1u32;
    let mut writer = WalWriter::new(
        stream_id,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        600_000_000_000,
    )
    .unwrap();

    let mut insert = OrderInsertedRecord {
        seq: 0,
        ts_ns: 1000,
        symbol_id: 1,
        user_id: 100,
        order_id_hi: 0,
        order_id_lo: 1,
        price: 100,
        qty: 10,
        side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
    };

    writer.append(&mut insert).unwrap();
    writer.flush().unwrap();

    let replay_addr: SocketAddr = "127.0.0.1:19202"
        .parse()
        .unwrap();
    let server =
        DxsReplayService::new(wal_dir, None).unwrap();

    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            server.serve(replay_addr).await
        })
    });
    thread::sleep(Duration::from_millis(100));

    let tip_file = tmp.path().join("tip");
    let result = run_replay_bootstrap_blocking(
        stream_id,
        replay_addr.to_string(),
        tip_file,
    )
    .unwrap();

    assert!(result.caught_up);

    let base_config = SymbolConfig {
        symbol_id: 0,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    };
    let mut state = MarketDataState::new(
        64,
        base_config,
        256,
        100,
    );

    for event in result.events {
        if let Some(rec) = event.insert {
            state.ensure_book(rec.symbol_id, rec.price);
            if let Some(book) = state.book_mut(rec.symbol_id)
            {
                book.apply_insert_by_id(
                    rec.price,
                    rec.qty,
                    rec.side,
                    rec.user_id,
                    rec.ts_ns,
                    rec.order_id_hi,
                    rec.order_id_lo,
                );
            }
        }
    }

    let snapshot_msg = state.snapshot_msg(1, 10);
    assert!(snapshot_msg.is_some());
    let msg = snapshot_msg.unwrap();
    assert!(msg.contains("\"B\""));
}
