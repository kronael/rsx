use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::BboRecord;
use rsx_dxs::records::CancelRequest;
use rsx_dxs::records::ConfigAppliedRecord;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderAcceptedRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::OrderFailedRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_CANCEL_REQUEST;
use rsx_dxs::records::RECORD_ORDER_REQUEST;
use rsx_dxs::wal::WalWriter;
use rsx_matching::config::load_applied_config;
use rsx_matching::config::poll_scheduled_configs;
use rsx_matching::config::write_applied_config;
use rsx_matching::wal_integration::flush_if_due;
use rsx_matching::wal_integration::load_snapshot;
use rsx_matching::wal_integration::save_snapshot;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_book::event::CANCEL_USER;
use rsx_book::event::REASON_CANCELLED;
use rsx_matching::wire::OrderMessage;
use rsx_types::NONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::install_panic_handler;
use rsx_types::SymbolConfig;
use rsx_types::time::time_ms;
use rsx_types::time::time_ns;
use rsx_matching::dedup::DedupTracker;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Instant;
use tokio_postgres::NoTls;
use tracing::error;
use tracing::info;
use tracing::warn;

const REASON_DUPLICATE: u8 = 3;

fn log_effective_matching_config(
    cfg: &SymbolConfig,
    db_url: &Option<String>,
    wal_dir: &str,
    me_addr: &SocketAddr,
    risk_addr: &SocketAddr,
    md_addr: &SocketAddr,
) {
    info!(
        "matching effective config: symbol_id={} tick_size={} lot_size={} price_decimals={} qty_decimals={} db_enabled={} wal_dir={} me_cmp_addr={} risk_cmp_addr={} md_cmp_addr={}",
        cfg.symbol_id,
        cfg.tick_size,
        cfg.lot_size,
        cfg.price_decimals,
        cfg.qty_decimals,
        db_url.is_some(),
        wal_dir,
        me_addr,
        risk_addr,
        md_addr,
    );
}

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

    let initial_config = match load_config_from_env() {
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

    let symbol_id = initial_config.symbol_id;

    // Database connection (optional, for config polling)
    let db_url = env::var("RSX_ME_DATABASE_URL").ok();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        // SAFETY: fail-fast at startup
        .expect("tokio runtime");

    let (pg_client, mut config_version) = if let Some(url) = &db_url {
        match rt.block_on(async {
            let (client, conn) = tokio_postgres::connect(url, NoTls)
                .await
                .map_err(|e| {
                    io::Error::other(
                        format!("db connect: {}", e),
                    )
                })?;
            tokio::spawn(async move {
                if let Err(e) = conn.await {
                    error!("db connection error: {}", e);
                }
            });
            Ok::<_, io::Error>(client)
        }) {
            Ok(client) => {
                let applied = rt.block_on(load_applied_config(&client, symbol_id));
                match applied {
                    Ok(Some(cfg)) => {
                        info!(
                            "loaded applied config v{} for symbol {}",
                            cfg.config_version, symbol_id
                        );
                        (Some(client), cfg.config_version)
                    }
                    Ok(None) => {
                        info!(
                            "no applied config found, using env config"
                        );
                        (Some(client), 0)
                    }
                    Err(e) => {
                        warn!("failed to load applied config: {}", e);
                        (Some(client), 0)
                    }
                }
            }
            Err(e) => {
                warn!("database unavailable: {}, using env config", e);
                (None, 0)
            }
        }
    } else {
        info!("no database url, config polling disabled");
        (None, 0)
    };

    let mut book = Orderbook::new(initial_config, 1024, 50_000);

    // WAL writer
    let wal_dir = env::var("RSX_ME_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".to_string());
    let mut wal_writer = WalWriter::new(
        symbol_id,
        &PathBuf::from(&wal_dir),
        None,
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create wal writer");

    // Restore book state from snapshot if available
    if let Some(loaded) = load_snapshot(&wal_dir, symbol_id) {
        book = *loaded;
        info!(
            "book restored from snapshot: seq={}",
            book.sequence,
        );
    } else {
        info!("no snapshot found, starting with empty book");
    }

    let mut last_flush = Instant::now();
    let mut last_snapshot = Instant::now();
    let mut last_config_poll = Instant::now();

    // CMP/UDP: receive orders from Risk
    let me_addr: SocketAddr = env::var("RSX_ME_CMP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9100".into())
        .parse()
        // SAFETY: fail-fast at startup
        .expect("invalid RSX_ME_CMP_ADDR");
    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_CMP_ADDR");
    // Risk's dedicated port for ME events (fills, BBO, etc.)
    let risk_me_recv_addr: SocketAddr =
        env::var("RSX_RISK_ME_RECV_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:28301".into())
            .parse()
            .expect("invalid RSX_RISK_ME_RECV_ADDR");

    let mut cmp_receiver = CmpReceiver::new(
        me_addr, risk_addr, symbol_id,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind CMP receiver");

    let mut cmp_sender = CmpSender::new(
        risk_me_recv_addr,
        symbol_id,
        &PathBuf::from(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create CMP sender");

    // CMP/UDP: send events to Marketdata
    let mkt_addr: SocketAddr =
        env::var("RSX_MD_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9103".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_MD_CMP_ADDR");
    let mut mkt_sender = CmpSender::new(
        mkt_addr,
        symbol_id,
        &PathBuf::from(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create MD CMP sender");
    log_effective_matching_config(
        &book.config,
        &db_url,
        &wal_dir,
        &me_addr,
        &risk_addr,
        &mkt_addr,
    );

    // DXS sidecar
    if let Ok(dxs_addr) = env::var("RSX_ME_DXS_ADDR") {
        let addr: std::net::SocketAddr = dxs_addr
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_ME_DXS_ADDR");
        let wal_path = PathBuf::from(&wal_dir);
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder
                ::new_multi_thread()
                .enable_all()
                .build()
                // SAFETY: fail-fast at startup
                .expect("tokio runtime for dxs");
            let service =
                rsx_dxs::DxsReplayService::new(wal_path, None)
                    // SAFETY: fail-fast at startup
                    .expect("failed to create dxs service");
            rt.block_on(async {
                service
                    .serve(addr)
                    .await
                    // SAFETY: fail-fast at startup
                    .expect("dxs server failed");
            });
        });
        info!(
            "dxs sidecar spawned on {}", dxs_addr
        );
    }

    // Emit CONFIG_APPLIED for this symbol on startup (if we have a version)
    if config_version > 0 {
        emit_config_applied(
            &mut wal_writer,
            &mut cmp_sender,
            &mut mkt_sender,
            symbol_id,
            config_version,
            0,
        );
    }

    let mut dedup = DedupTracker::new();

    info!("matching engine started");

    loop {
        // Receive orders/cancels via CMP/UDP from Risk
        if let Some((hdr, payload)) =
            cmp_receiver.try_recv()
        {
            if hdr.record_type == RECORD_ORDER_REQUEST
                && payload.len()
                    >= std::mem::size_of::<
                        OrderMessage,
                    >()
            {
                let order_msg = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const OrderMessage,
                    )
                };
                // Dedup check
                let is_dup = dedup.check_and_insert(
                    order_msg.user_id,
                    order_msg.order_id_hi,
                    order_msg.order_id_lo,
                );

                if is_dup {
                    let ts = time_ns();
                    let mut fail = OrderFailedRecord {
                        seq: 0,
                        ts_ns: ts,
                        user_id: order_msg.user_id,
                        _pad0: 0,
                        order_id_hi: order_msg
                            .order_id_hi,
                        order_id_lo: order_msg
                            .order_id_lo,
                        reason: REASON_DUPLICATE,
                        _pad: [0; 23],
                    };
                    let _ =
                        wal_writer.append(&mut fail);
                    let _ =
                        cmp_sender.send(&mut fail);
                } else {
                    // Record acceptance in WAL
                    let ts = time_ns();
                    let mut accepted =
                        OrderAcceptedRecord {
                            seq: 0,
                            ts_ns: ts,
                            user_id: order_msg.user_id,
                            symbol_id,
                            order_id_hi: order_msg
                                .order_id_hi,
                            order_id_lo: order_msg
                                .order_id_lo,
                            price: order_msg.price,
                            qty: order_msg.qty,
                            side: order_msg.side,
                            tif: order_msg.tif,
                            reduce_only: order_msg
                                .reduce_only,
                            post_only: order_msg
                                .post_only,
                            _pad1: [0; 12],
                        };
                    let _ = wal_writer
                        .append(&mut accepted);

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

                    // Send events to Risk (all)
                    for event in book.events() {
                        let _ = send_event_cmp(
                            &mut cmp_sender,
                            event,
                            symbol_id,
                            ts_ns,
                        );
                    }

                    // Send to Marketdata (no OrderDone)
                    for event in book.events() {
                        let _ =
                            send_event_marketdata(
                                &mut mkt_sender,
                                event,
                                symbol_id,
                                ts_ns,
                            );
                    }
                }
            } else if hdr.record_type
                == RECORD_CANCEL_REQUEST
                && payload.len()
                    >= std::mem::size_of::<
                        CancelRequest,
                    >()
            {
                let req = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const CancelRequest,
                    )
                };
                process_cancel(
                    &mut book,
                    &mut wal_writer,
                    &mut cmp_sender,
                    &mut mkt_sender,
                    symbol_id,
                    req.user_id,
                    req.order_id_hi,
                    req.order_id_lo,
                );
            }
        } else if book.is_migrating() {
            book.migrate_batch(100);
        }

        dedup.maybe_cleanup();

        // Poll config every 10 minutes
        if last_config_poll.elapsed().as_secs() >= 600 {
            if let Some(ref client) = pg_client {
                let now_ms = time_ms();
                match rt.block_on(poll_scheduled_configs(
                    client,
                    symbol_id,
                    config_version,
                    now_ms,
                )) {
                    Ok(configs) => {
                        for cfg in configs {
                            let new_config = cfg.to_symbol_config(symbol_id);
                            book.update_config(new_config);
                            config_version = cfg.config_version;
                            let ts = time_ns();
                            let _ = rt.block_on(write_applied_config(
                                client,
                                symbol_id,
                                &cfg,
                                ts,
                            ));
                            emit_config_applied(
                                &mut wal_writer,
                                &mut cmp_sender,
                                &mut mkt_sender,
                                symbol_id,
                                cfg.config_version,
                                cfg.effective_at_ms,
                            );
                        }
                    }
                    Err(e) => {
                        warn!("config poll failed: {}", e);
                    }
                }
            }
            last_config_poll = Instant::now();
        }

        let _ = flush_if_due(
            &mut wal_writer, &mut last_flush,
        );

        // Save snapshot every 10s
        if last_snapshot.elapsed().as_secs() >= 10 {
            if let Err(e) = save_snapshot(
                &book, &wal_dir, symbol_id,
            ) {
                warn!("snapshot save: {}", e);
            }
            last_snapshot = Instant::now();
        }

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
                price,
                qty,
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
                price,
                qty,
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
                remaining_qty,
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
                filled_qty,
                remaining_qty,
                final_status: reason,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
            };
            let _ = sender.send(&mut record)?;
        }
        rsx_book::event::Event::BBO {
            bid_px,
            bid_qty,
            ask_px,
            ask_qty,
        } => {
            let mut record = BboRecord {
                seq: 0,
                ts_ns,
                symbol_id,
                _pad0: 0,
                bid_px,
                bid_qty,
                bid_count: 0,
                _pad1: 0,
                ask_px,
                ask_qty,
                ask_count: 0,
                _pad2: 0,
            };
            let _ = sender.send(&mut record)?;
        }
        rsx_book::event::Event::OrderFailed { .. } => {}
    }
    Ok(())
}

fn emit_config_applied(
    wal: &mut WalWriter,
    risk_sender: &mut CmpSender,
    mkt_sender: &mut CmpSender,
    symbol_id: u32,
    config_version: u64,
    effective_at_ms: u64,
) {
    let ts = time_ns();
    let mut record = ConfigAppliedRecord {
        seq: 0,
        ts_ns: ts,
        symbol_id,
        _pad0: 0,
        config_version,
        effective_at_ms,
        applied_at_ns: ts,
    };
    let _ = wal.append(&mut record);
    let _ = risk_sender.send(&mut record);
    let _ = mkt_sender.send(&mut record);
    info!(
        "emitted config_applied v{} for symbol {}",
        config_version, symbol_id,
    );
}

/// Cancel a resting order by order_id, emit events,
/// write WAL, and send CMP to risk + marketdata.
fn process_cancel(
    book: &mut Orderbook,
    wal_writer: &mut WalWriter,
    cmp_sender: &mut CmpSender,
    mkt_sender: &mut CmpSender,
    symbol_id: u32,
    user_id: u32,
    order_id_hi: u64,
    order_id_lo: u64,
) {
    // Find the slab handle by scanning active orders.
    // Slab is bounded (capacity=1024), so linear scan
    // is acceptable on the hot path for cancels.
    let cap = book.orders.len();
    let mut found = NONE;
    for i in 0..cap {
        let slot = book.orders.get(i);
        if slot.is_active()
            && slot.user_id == user_id
            && slot.order_id_hi == order_id_hi
            && slot.order_id_lo == order_id_lo
        {
            found = i;
            break;
        }
    }
    if found == NONE {
        warn!(
            "cancel: order not found \
             user={} id={:#x}/{:#x}",
            user_id, order_id_hi, order_id_lo,
        );
        return;
    }

    let slot = book.orders.get(found);
    let remaining_qty = slot.remaining_qty;
    let old_bid = book.best_bid_tick;
    let old_ask = book.best_ask_tick;

    book.event_len = 0;

    book.cancel_order(found);

    book.emit(rsx_book::event::Event::OrderCancelled {
        handle: found,
        user_id,
        remaining_qty,
        reason: CANCEL_USER,
        order_id_hi,
        order_id_lo,
    });
    book.emit(rsx_book::event::Event::OrderDone {
        handle: found,
        user_id,
        reason: REASON_CANCELLED,
        filled_qty: Qty(0),
        remaining_qty,
        order_id_hi,
        order_id_lo,
    });

    // Emit BBO if best bid or ask changed
    if book.best_bid_tick != old_bid
        || book.best_ask_tick != old_ask
    {
        let (bid_px, bid_qty) =
            if book.best_bid_tick != NONE {
                let lvl = &book.active_levels
                    [book.best_bid_tick as usize];
                let px =
                    book.orders.get(lvl.head).price.0;
                (px, lvl.total_qty)
            } else {
                (0, 0)
            };
        let (ask_px, ask_qty) =
            if book.best_ask_tick != NONE {
                let lvl = &book.active_levels
                    [book.best_ask_tick as usize];
                let px =
                    book.orders.get(lvl.head).price.0;
                (px, lvl.total_qty)
            } else {
                (0, 0)
            };
        book.emit(rsx_book::event::Event::BBO {
            bid_px: Price(bid_px),
            bid_qty: Qty(bid_qty),
            ask_px: Price(ask_px),
            ask_qty: Qty(ask_qty),
        });
    }

    let ts_ns = time_ns();
    let _ = write_events_to_wal(
        wal_writer, book, symbol_id, ts_ns,
    );
    for event in book.events() {
        let _ = send_event_cmp(
            cmp_sender, event, symbol_id, ts_ns,
        );
    }
    for event in book.events() {
        let _ = send_event_marketdata(
            mkt_sender, event, symbol_id, ts_ns,
        );
    }
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
