use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::ConfigAppliedRecord;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::wal::WalWriter;
use rsx_matching::wal_integration::flush_if_due;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_matching::wire::OrderMessage;
use rsx_types::install_panic_handler;
use rsx_types::SymbolConfig;
use rsx_types::time::time_ns;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Instant;
use tracing::info;

fn get_env_u32(key: &str) -> io::Result<u32> {
    let raw = env::var(key).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing env var {}", key),
        )
    })?;
    raw.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {}: {}", key, raw),
        )
    })
}

fn get_env_u8(key: &str) -> io::Result<u8> {
    let raw = env::var(key).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing env var {}", key),
        )
    })?;
    raw.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {}: {}", key, raw),
        )
    })
}

fn get_env_i64(key: &str) -> io::Result<i64> {
    let raw = env::var(key).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing env var {}", key),
        )
    })?;
    raw.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {}: {}", key, raw),
        )
    })
}

fn load_config_from_env() -> io::Result<SymbolConfig> {
    let symbol_id = get_env_u32("RSX_ME_SYMBOL_ID")?;
    let price_decimals =
        get_env_u8("RSX_ME_PRICE_DECIMALS")?;
    let qty_decimals =
        get_env_u8("RSX_ME_QTY_DECIMALS")?;
    let tick_size = get_env_i64("RSX_ME_TICK_SIZE")?;
    let lot_size = get_env_i64("RSX_ME_LOT_SIZE")?;
    Ok(SymbolConfig {
        symbol_id,
        price_decimals,
        qty_decimals,
        tick_size,
        lot_size,
    })
}

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = match load_config_from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {}", e);
            std::process::exit(1);
        }
    };

    // Pin to core if specified
    if let Ok(core_str) = env::var("RSX_ME_CORE_ID") {
        if let Ok(core_id) = core_str.parse::<usize>() {
            let ids = core_affinity::get_core_ids()
                .unwrap_or_default();
            if let Some(id) = ids.get(core_id) {
                core_affinity::set_for_current(*id);
                info!("pinned to core {}", core_id);
            }
        }
    }

    let symbol_id = config.symbol_id;
    let mut book =
        Orderbook::new(config, 1024, 50_000);

    // WAL writer
    let wal_dir = env::var("RSX_ME_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".to_string());
    let mut wal_writer = WalWriter::new(
        symbol_id,
        &PathBuf::from(&wal_dir),
        64 * 1024 * 1024,       // 64MB rotation
        10 * 60 * 1_000_000_000, // 10min retention
    )
    .expect("failed to create wal writer");
    let mut last_flush = Instant::now();

    // CMP/UDP: receive orders from Risk
    let me_addr: SocketAddr = env::var("RSX_ME_CMP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9100".into())
        .parse()
        .expect("invalid RSX_ME_CMP_ADDR");
    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            .expect("invalid RSX_RISK_CMP_ADDR");

    let mut cmp_receiver = CmpReceiver::new(
        me_addr, risk_addr, symbol_id,
    )
    .expect("failed to bind CMP receiver");

    let mut cmp_sender = CmpSender::new(
        risk_addr, symbol_id, &PathBuf::from(&wal_dir),
    )
    .expect("failed to create CMP sender");

    // CMP/UDP: send events to Marketdata
    let mkt_addr: SocketAddr =
        env::var("RSX_MD_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9103".into())
            .parse()
            .expect("invalid RSX_MD_CMP_ADDR");
    let mut mkt_sender = CmpSender::new(
        mkt_addr,
        symbol_id,
        &PathBuf::from(&wal_dir),
    )
    .expect("failed to create MD CMP sender");

    // DXS sidecar
    if let Ok(dxs_addr) = env::var("RSX_ME_DXS_ADDR") {
        let addr: std::net::SocketAddr = dxs_addr
            .parse()
            .expect("invalid RSX_ME_DXS_ADDR");
        let wal_path = PathBuf::from(&wal_dir);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder
                ::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime for dxs");
            let service =
                rsx_dxs::DxsReplayService::new(wal_path);
            rt.block_on(async {
                service
                    .serve(addr)
                    .await
                    .expect("dxs server failed");
            });
        });
        info!(
            "dxs sidecar spawned on {}", dxs_addr
        );
    }

    // Emit CONFIG_APPLIED for this symbol on startup
    emit_startup_config(
        &mut wal_writer,
        &mut cmp_sender,
        symbol_id,
    );

    info!("matching engine started");

    loop {
        // Receive orders via CMP/UDP from Risk
        if let Some((_hdr, payload)) =
            cmp_receiver.try_recv()
        {
            // Decode OrderMessage from payload
            if payload.len()
                >= std::mem::size_of::<OrderMessage>()
            {
                let order_msg = unsafe {
                    std::ptr::read(
                        payload.as_ptr()
                            as *const OrderMessage,
                    )
                };
                let mut incoming =
                    order_msg.to_incoming();
                process_new_order(
                    &mut book, &mut incoming,
                );

                // Write events to WAL
                let ts_ns = time_ns();
                let _ = write_events_to_wal(
                    &mut wal_writer,
                    &book,
                    symbol_id,
                    ts_ns,
                );

                // Send events to Risk (all types)
                for event in book.events() {
                    let _ = send_event_cmp(
                        &mut cmp_sender,
                        event,
                        symbol_id,
                        ts_ns,
                    );
                }

                // Send events to Marketdata (no OrderDone)
                for event in book.events() {
                    let _ = send_event_marketdata(
                        &mut mkt_sender,
                        event,
                        symbol_id,
                        ts_ns,
                    );
                }
            }
        } else if book.is_migrating() {
            book.migrate_batch(100);
        }

        let _ = flush_if_due(
            &mut wal_writer, &mut last_flush,
        );
        let _ = cmp_sender.tick();
        let _ = mkt_sender.tick();
        cmp_receiver.tick();
        cmp_sender.recv_control();
        mkt_sender.recv_control();
    }
}

fn send_event_cmp(
    sender: &mut CmpSender,
    event: &rsx_book::event::Event,
    symbol_id: u32,
    ts_ns: u64,
) -> io::Result<()> {
    match *event {
        rsx_book::event::Event::Fill {
            maker_user_id,
            taker_user_id,
            price,
            qty,
            side,
            maker_order_id_hi,
            maker_order_id_lo,
            taker_order_id_hi,
            taker_order_id_lo,
            ..
        } => {
            let mut record = FillRecord {
                seq: 0,
                ts_ns,
                symbol_id,
                taker_user_id,
                maker_user_id,
                _pad0: 0,
                taker_order_id_hi,
                taker_order_id_lo,
                maker_order_id_hi,
                maker_order_id_lo,
                price: price.0,
                qty: qty.0,
                taker_side: side,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
            };
            let _ = sender.send(&mut record)?;
        }
        rsx_book::event::Event::OrderInserted {
            user_id,
            side,
            price,
            qty,
            order_id_hi,
            order_id_lo,
            ..
        } => {
            let mut record = OrderInsertedRecord {
                seq: 0,
                ts_ns,
                symbol_id,
                user_id,
                order_id_hi,
                order_id_lo,
                price: price.0,
                qty: qty.0,
                side,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
            };
            let _ = sender.send(&mut record)?;
        }
        rsx_book::event::Event::OrderCancelled {
            user_id,
            remaining_qty,
            reason,
            order_id_hi,
            order_id_lo,
            ..
        } => {
            let mut record = OrderCancelledRecord {
                seq: 0,
                ts_ns,
                symbol_id,
                user_id,
                order_id_hi,
                order_id_lo,
                remaining_qty: remaining_qty.0,
                reason,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
            };
            let _ = sender.send(&mut record)?;
        }
        rsx_book::event::Event::OrderDone {
            user_id,
            reason,
            filled_qty,
            remaining_qty,
            order_id_hi,
            order_id_lo,
            ..
        } => {
            let mut record = OrderDoneRecord {
                seq: 0,
                ts_ns,
                symbol_id,
                user_id,
                order_id_hi,
                order_id_lo,
                filled_qty: filled_qty.0,
                remaining_qty: remaining_qty.0,
                final_status: reason,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
            };
            let _ = sender.send(&mut record)?;
        }
        _ => {}
    }
    Ok(())
}

fn emit_startup_config(
    wal: &mut WalWriter,
    risk_sender: &mut CmpSender,
    symbol_id: u32,
) {
    let ts = time_ns();
    let mut record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id,
        _pad0: 0,
        config_version: 1,
        effective_at_ms: 0,
        applied_at_ns: ts,
    };
    let _ = wal.append(&mut record);
    let _ = risk_sender.send(&mut record);
    info!(
        "emitted config_applied for symbol {}",
        symbol_id,
    );
}

/// Send events to Marketdata -- Fill, OrderInserted,
/// OrderCancelled only. OrderDone excluded per MD20.
fn send_event_marketdata(
    sender: &mut CmpSender,
    event: &rsx_book::event::Event,
    symbol_id: u32,
    ts_ns: u64,
) -> io::Result<()> {
    match *event {
        rsx_book::event::Event::Fill { .. }
        | rsx_book::event::Event::OrderInserted { .. }
        | rsx_book::event::Event::OrderCancelled {
            ..
        } => send_event_cmp(sender, event, symbol_id, ts_ns),
        _ => Ok(()),
    }
}
