use rsx_cast::ReplicationService;
use rsx_cast::WalWriter;
use rsx_marketdata::replay::run_replay_bootstrap;
use rsx_marketdata::state::MarketDataState;
use rsx_marketdata::types::L2Snapshot;
use rsx_marketdata::wire::encode_l2_snapshot;
use rsx_messages::FillRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::time::Duration;
use tempfile::TempDir;

fn reserve_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

/// Self-signed cert used as BOTH server identity and client CA
/// (single-box self-trust). Replication is TLS-mandatory.
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

/// Async test helper. Runs on a dedicated thread with
/// enough stack for debug-mode async state machines.
fn run_async_test<F>(f: F)
where
    F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>> + Send + 'static,
{
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(f())
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn dxs_replay_rebuilds_shadow_book() {
    run_async_test(|| {
        Box::pin(async {
            let replay_addr = reserve_port();
            let tmp = TempDir::new().unwrap();
            let wal_dir = tmp.path().join("wal");
            std::fs::create_dir_all(&wal_dir).unwrap();
            let tls = test_tls(tmp.path());

            let stream_id = 1u32;
            let mut writer = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();

            let mut insert1 = OrderInsertedRecord {
                seq: 0,
                ts_ns: 1000,
                symbol_id: 1,
                user_id: 100,
                order_id_hi: 0,
                order_id_lo: 1,
                price: Price(100),
                qty: Qty(10),
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
                price: Price(101),
                qty: Qty(20),
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
                price: Price(100),
                qty: Qty(5),
                taker_side: 1,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
                taker_ts_ns: 0,
            };

            {
                let framed = writer.prepare(&mut insert1).unwrap();
                writer.append_framed(&framed).unwrap();
            }
            {
                let framed = writer.prepare(&mut insert2).unwrap();
                writer.append_framed(&framed).unwrap();
            }
            {
                let framed = writer.prepare(&mut fill).unwrap();
                writer.append_framed(&framed).unwrap();
            }
            writer.flush().unwrap();

            let server = ReplicationService::new(wal_dir, tls.clone()).unwrap();

            tokio::spawn(async move { server.serve(replay_addr).await });
            tokio::time::sleep(Duration::from_millis(100)).await;

            let tip_file = tmp.path().join("tip");
            let result = run_replay_bootstrap(stream_id, replay_addr.to_string(), tip_file, tls)
                .await
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
            let mut state = MarketDataState::new(64, base_config, 256, 100);

            for event in result.events {
                if let Some(rec) = event.insert {
                    state.ensure_book(rec.symbol_id, rec.price.0);
                    if let Some(book) = state.book_mut(rec.symbol_id) {
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
                } else if let Some(rec) = event.fill {
                    if let Some(book) = state.book_mut(rec.symbol_id) {
                        book.apply_fill_by_order_id(
                            rec.maker_order_id_hi,
                            rec.maker_order_id_lo,
                            rec.qty.0,
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
        })
    });
}

#[test]
fn recovery_from_me_wal_then_live() {
    run_async_test(|| {
        Box::pin(async {
            let replay_addr = reserve_port();
            let tmp = TempDir::new().unwrap();
            let wal_dir = tmp.path().join("wal");
            std::fs::create_dir_all(&wal_dir).unwrap();
            let tls = test_tls(tmp.path());

            let stream_id = 1u32;
            let mut writer = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();

            let mut insert = OrderInsertedRecord {
                seq: 0,
                ts_ns: 1000,
                symbol_id: 1,
                user_id: 100,
                order_id_hi: 0,
                order_id_lo: 1,
                price: Price(100),
                qty: Qty(10),
                side: 0,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
            };

            {
                let framed = writer.prepare(&mut insert).unwrap();
                writer.append_framed(&framed).unwrap();
            }
            writer.flush().unwrap();

            let server = ReplicationService::new(wal_dir, tls.clone()).unwrap();

            tokio::spawn(async move { server.serve(replay_addr).await });
            tokio::time::sleep(Duration::from_millis(100)).await;

            let tip_file = tmp.path().join("tip");
            let result = run_replay_bootstrap(stream_id, replay_addr.to_string(), tip_file, tls)
                .await
                .unwrap();

            assert!(result.caught_up);
            assert_eq!(result.last_seq, 1);
        })
    });
}

#[test]
fn recovery_snapshot_sent_after_catchup() {
    run_async_test(|| {
        Box::pin(async {
            let replay_addr = reserve_port();
            let tmp = TempDir::new().unwrap();
            let wal_dir = tmp.path().join("wal");
            std::fs::create_dir_all(&wal_dir).unwrap();
            let tls = test_tls(tmp.path());

            let stream_id = 1u32;
            let mut writer = WalWriter::new(stream_id, &wal_dir, 64 * 1024 * 1024).unwrap();

            let mut insert = OrderInsertedRecord {
                seq: 0,
                ts_ns: 1000,
                symbol_id: 1,
                user_id: 100,
                order_id_hi: 0,
                order_id_lo: 1,
                price: Price(100),
                qty: Qty(10),
                side: 0,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
            };

            {
                let framed = writer.prepare(&mut insert).unwrap();
                writer.append_framed(&framed).unwrap();
            }
            writer.flush().unwrap();

            let server = ReplicationService::new(wal_dir, tls.clone()).unwrap();

            tokio::spawn(async move { server.serve(replay_addr).await });
            tokio::time::sleep(Duration::from_millis(100)).await;

            let tip_file = tmp.path().join("tip");
            let result = run_replay_bootstrap(stream_id, replay_addr.to_string(), tip_file, tls)
                .await
                .unwrap();

            assert!(result.caught_up);

            let base_config = SymbolConfig {
                symbol_id: 0,
                price_decimals: 0,
                qty_decimals: 0,
                tick_size: 1,
                lot_size: 1,
            };
            let mut state = MarketDataState::new(64, base_config, 256, 100);

            for event in result.events {
                if let Some(rec) = event.insert {
                    state.ensure_book(rec.symbol_id, rec.price.0);
                    if let Some(book) = state.book_mut(rec.symbol_id) {
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
            }

            let snapshot_msg = state.snapshot_msg(1, 10);
            assert!(snapshot_msg.is_some());
            let msg = snapshot_msg.unwrap();
            // Protobuf snapshot frame: [len:4 BE][MdFrame body]; body byte
            // 0 is the Snapshot oneof tag (field 2, wire type 2 = 0x12).
            // The replayed book yields a larger frame than an empty book.
            let empty = encode_l2_snapshot(&L2Snapshot {
                symbol_id: 1,
                bids: Vec::new(),
                asks: Vec::new(),
                timestamp_ns: 0,
                seq: 0,
            });
            assert_eq!(msg[4], 18, "snapshot uses the Snapshot oneof tag");
            assert!(msg.len() > empty.len(), "replayed snapshot carries levels");
        })
    });
}
