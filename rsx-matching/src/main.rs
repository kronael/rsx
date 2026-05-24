use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_dxs::cmp::CmpRecv;
use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_messages::BboRecord;
use rsx_messages::CancelRequest;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_dxs::wal::WalWriter;
use rsx_matching::config::load_applied_config;
use rsx_matching::config::poll_scheduled_configs;
use rsx_matching::config::write_applied_config;
use rsx_matching::wal_integration::flush_if_due;
use rsx_matching::wal_integration::load_snapshot;
use rsx_matching::wal_integration::load_wal_seq;
use rsx_matching::wal_integration::replay_wal_after_snapshot;
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
use rustc_hash::FxHashMap;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tokio_postgres::NoTls;
use tracing::error;
use tracing::info;
use tracing::warn;

const REASON_DUPLICATE: u8 = 3;

/// Key into the order index. The matching engine maintains
/// `FxHashMap<OrderKey, slab_handle: u32>` so cancels are
/// O(1) instead of an O(n) slab scan. Updated from
/// `book.events()` after every match cycle: OrderInserted
/// adds, OrderDone removes.
type OrderKey = (u32, u64, u64);

/// After a match cycle, walk `book.events()` once and keep
/// the order index in sync. Insert on OrderInserted, remove
/// on OrderDone (which fires for every terminal transition,
/// including fully-filled and cancelled).
fn update_order_index(
    events: &[rsx_book::event::Event],
    index: &mut FxHashMap<OrderKey, u32>,
) {
    for event in events {
        match *event {
            rsx_book::event::Event::OrderInserted {
                handle,
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.insert(
                    (user_id, order_id_hi, order_id_lo),
                    handle,
                );
            }
            rsx_book::event::Event::OrderDone {
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.remove(&(
                    user_id,
                    order_id_hi,
                    order_id_lo,
                ));
            }
            _ => {}
        }
    }
}

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

    // Drain hot-path latency samples out-of-band
    // (see rsx-types/src/latency.rs). 100 ms is a
    // good compromise between dashboard freshness
    // and drain-thread CPU.
    rsx_log::start_drainer(100);

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

    let mut book = Orderbook::new(initial_config, 65_536, 50_000);

    // WAL writer.
    // Hot retention is 4 h. Long-term durability is the
    // archive's job (see ARCHIVE setup in 48-wal.md); hot
    // WAL just needs enough window to absorb a crash and
    // a snapshot-to-replay gap. 4 h ≫ 10 s snapshot
    // cadence with margin for a multi-hour ops outage.
    let wal_dir = env::var("RSX_ME_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".to_string());
    let mut wal_writer = WalWriter::new(
        symbol_id,
        &PathBuf::from(&wal_dir),
        None,
        64 * 1024 * 1024,
        4 * 60 * 60 * 1_000_000_000,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create wal writer");

    let mut dedup = DedupTracker::new();
    let mut order_index: FxHashMap<OrderKey, u32> =
        FxHashMap::default();

    // Restore book state from snapshot if available.
    // Recovery strategy (R-N1):
    //   1. Load snapshot.bin into `book` (if present).
    //   2. Read wal_seq.txt sidecar — the WAL seq at the
    //      moment the snapshot was taken.
    //   3. Replay WAL records seq > sidecar into the book by
    //      re-executing OrderAccepted via process_new_order;
    //      this regenerates fills + emitted events deterministically.
    //   4. Bump wal_writer.next_seq so subsequent live writes
    //      don't collide with replayed seqs already on disk.
    let snapshot_loaded =
        load_snapshot(&wal_dir, symbol_id);
    let replay_from = if let Some(loaded) = snapshot_loaded {
        book = *loaded;
        let sidecar = load_wal_seq(&wal_dir, symbol_id);
        info!(
            "book restored from snapshot: book.seq={} wal_seq_sidecar={:?}",
            book.sequence, sidecar,
        );
        // If sidecar missing (legacy snapshot), full WAL replay
        // would duplicate the orders the snapshot already
        // contains. Safer to skip replay; that re-introduces
        // R-N1 for legacy snapshots only, which die after the
        // first new save_snapshot anyway.
        sidecar.map(|s| s + 1)
    } else {
        info!(
            "no snapshot found — replaying WAL from seq 1"
        );
        Some(1)
    };
    if let Some(start_seq) = replay_from {
        match replay_wal_after_snapshot(
            &mut book,
            &mut order_index,
            &mut dedup,
            &wal_dir,
            symbol_id,
            start_seq,
        ) {
            Ok(last_seq) if last_seq >= start_seq => {
                wal_writer.set_next_seq(last_seq + 1);
            }
            Ok(_) => {
                // No records replayed — leave next_seq=1
                // (writer is fresh).
            }
            Err(e) => {
                warn!(
                    "wal replay failed: {} — \
                     continuing with snapshot-only state",
                    e,
                );
            }
        }
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
    // NAK destination: risk's ME sender bind addr
    // (RSX_RISK_ME_SEND_ADDR), with RSX_RISK_CMP_ADDR as fallback.
    let risk_nak_addr: SocketAddr =
        env::var("RSX_RISK_ME_SEND_ADDR")
            .or_else(|_| env::var("RSX_RISK_CMP_ADDR"))
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid NAK sender addr");
    // Risk's dedicated port for ME events (fills, BBO, etc.)
    let risk_me_recv_addr: SocketAddr =
        env::var("RSX_RISK_ME_RECV_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:28301".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_ME_RECV_ADDR");

    let mut cmp_receiver = CmpReceiver::new(
        me_addr, risk_nak_addr, symbol_id,
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
        &risk_nak_addr,
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

    // `dedup` + `order_index` are populated above by
    // replay_wal_after_snapshot when applicable. After this
    // point only the live event loop mutates them.

    // Graceful shutdown: SIGTERM/SIGINT flips the flag,
    // the main loop notices and drains the WAL writer
    // before exiting. Required so a parent process leaves
    // the active WAL persisted (spec invariant 7) — the
    // panic handler is left untouched so real crashes
    // still surface.
    static SHUTDOWN: AtomicBool = AtomicBool::new(false);
    extern "C" fn on_signal(_: libc::c_int) {
        SHUTDOWN.store(true, Ordering::SeqCst);
    }
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            on_signal as *const () as libc::sighandler_t,
        );
    }

    info!("matching engine started");

    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("shutdown signal received, draining wal");
            if let Err(e) = wal_writer.flush() {
                error!("shutdown wal flush failed: {e}");
            }
            if let Err(e) = save_snapshot(
                &book,
                &wal_dir,
                symbol_id,
                wal_writer.last_seq(),
            ) {
                warn!("shutdown snapshot save failed: {e}");
            }
            info!("matching engine shutdown complete");
            std::process::exit(0);
        }
        // Receive orders/cancels via CMP/UDP from Risk
        let recv = cmp_receiver.try_recv();
        if let CmpRecv::Faulted {
            last_delivered_seq,
            gap_start,
            gap_end_inclusive,
        } = recv
        {
            // Per CMP v4 contract: FAULTED means an
            // unrecoverable gap inside the in-band recovery
            // horizon. Recover out-of-band via DXS/TCP replay
            // from `last_delivered_seq + 1`, then reset the
            // CMP receiver and resume live UDP delivery.
            warn!(
                "matching tile FAULTED at seq={}, opening \
                 DXS replay from seq={} (gap=[{}..={}])",
                last_delivered_seq,
                last_delivered_seq + 1,
                gap_start,
                gap_end_inclusive,
            );
            let replay_addr = env::var(
                "RSX_ME_REPLAY_DXS_ADDR",
            )
            .expect(
                "FAULTED requires RSX_ME_REPLAY_DXS_ADDR \
                 pointing at the risk producer's DXS server",
            );
            let tip_file = PathBuf::from(&wal_dir).join(
                format!(
                    "me_{}_replay_tip.bin", symbol_id,
                ),
            );
            let new_tip = rsx_matching::replay::drain_dxs_replay_into_book(
                &rt,
                &mut book,
                &mut order_index,
                &mut dedup,
                &mut wal_writer,
                symbol_id,
                replay_addr,
                last_delivered_seq,
                tip_file,
            )
            // SAFETY: drain failure is unrecoverable for
            // the POC — supervisor restarts will then take
            // the normal cold-start replay path. Production
            // wiring should add bounded retries here.
            .expect("dxs replay drain failed");
            cmp_receiver.reset_after_replay(new_tip);
            info!(
                "matching tile recovered via DXS replay, \
                 new_tip={}, resuming live UDP",
                new_tip,
            );
            continue;
        }
        if let CmpRecv::Data(hdr, payload) = recv {
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
                // F4.3 — per-stage latency trace. Stage
                // `me_in` = order arrived at matching engine.
                // t_us measured against gateway submit ts.
                {
                    let now_ns = time_ns();
                    let t_us = now_ns
                        .saturating_sub(order_msg.timestamp_ns)
                        / 1000;
                    rsx_log::latency::sample("me_in", order_msg.order_id_hi, order_msg.order_id_lo, t_us, order_msg.timestamp_ns);
                }
                // Dedup check
                let is_dup = dedup.check_and_insert(
                    order_msg.user_id,
                    order_msg.order_id_hi,
                    order_msg.order_id_lo,
                );
                // Sub-stage: dedup completed (after FxHashMap
                // insert). Anchored on the same t0 as me_in.
                {
                    let now_ns = time_ns();
                    let t_us = now_ns
                        .saturating_sub(order_msg.timestamp_ns)
                        / 1000;
                    rsx_log::latency::sample("me_dedup_done", order_msg.order_id_hi, order_msg.order_id_lo, t_us, order_msg.timestamp_ns);
                }

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
                    wal_writer
                        .append(&mut fail)
                        .expect("wal append failed (duplicate-reject) — violates 6-consistency.md invariant 7 (matching engine persists orderbook via snapshot + WAL)");
                    if let Err(e) = cmp_sender.send(&mut fail) {
                        warn!("cmp send fail-record (duplicate): {e}");
                    }
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
                            cid: [0; 20],
                        };
                    wal_writer
                        .append(&mut accepted)
                        .expect("wal append failed (order-accepted) — violates 6-consistency.md invariant 7 (WAL persistence) and breaks dedup on replay");
                    // Sub-stage: OrderAcceptedRecord appended.
                    {
                        let now_ns = time_ns();
                        let t_us = now_ns
                            .saturating_sub(
                                order_msg.timestamp_ns,
                            )
                            / 1000;
                        rsx_log::latency::sample("me_wal_accepted_done", order_msg.order_id_hi, order_msg.order_id_lo, t_us, order_msg.timestamp_ns);
                    }

                    let mut incoming =
                        order_msg.to_incoming();
                    process_new_order(
                        &mut book, &mut incoming,
                    );
                    // Sub-stage: match cycle finished.
                    {
                        let now_ns = time_ns();
                        let t_us = now_ns
                            .saturating_sub(
                                order_msg.timestamp_ns,
                            )
                            / 1000;
                        rsx_log::latency::sample("me_match_done", order_msg.order_id_hi, order_msg.order_id_lo, t_us, order_msg.timestamp_ns);
                    }

                    // Write events to WAL — authoritative,
                    // crash on failure rather than lose fills.
                    let ts_ns = time_ns();
                    write_events_to_wal(
                        &mut wal_writer,
                        &book,
                        symbol_id,
                        ts_ns,
                    )
                    .expect("wal append failed (event path) — violates 6-consistency.md invariant 1 (totally-ordered events) and ordering rule 'Fills precede ORDER_DONE' (§2)");
                    // Sub-stage: events flushed to WAL.
                    {
                        let now_ns = time_ns();
                        let t_us = now_ns
                            .saturating_sub(
                                order_msg.timestamp_ns,
                            )
                            / 1000;
                        rsx_log::latency::sample("me_wal_events_done", order_msg.order_id_hi, order_msg.order_id_lo, t_us, order_msg.timestamp_ns);
                    }

                    // Maintain the (user, oid) -> handle
                    // index so subsequent cancels are O(1).
                    update_order_index(
                        book.events(),
                        &mut order_index,
                    );
                    // Sub-stage: order index updated.
                    {
                        let now_ns = time_ns();
                        let t_us = now_ns
                            .saturating_sub(
                                order_msg.timestamp_ns,
                            )
                            / 1000;
                        rsx_log::latency::sample("me_index_done", order_msg.order_id_hi, order_msg.order_id_lo, t_us, order_msg.timestamp_ns);
                    }

                    // F4.3 — per-stage latency trace. Stage
                    // `me_out` = ME finished matching and is
                    // about to forward events to risk. t_us
                    // measured against the order's gateway
                    // submit timestamp. We only log once per
                    // incoming order (against its oid).
                    {
                        let now_ns = time_ns();
                        let t_us = now_ns
                            .saturating_sub(
                                order_msg.timestamp_ns,
                            )
                            / 1000;
                        rsx_log::latency::sample("me_out", order_msg.order_id_hi, order_msg.order_id_lo, t_us, order_msg.timestamp_ns);
                    }
                    // CMP sends are best-effort: receivers
                    // recover via NAK / TCP replay.
                    for event in book.events() {
                        if let Err(e) = send_event_cmp(
                            &mut cmp_sender,
                            event,
                            symbol_id,
                            ts_ns,
                        ) {
                            warn!("cmp send event to risk failed: {e}");
                        }
                    }

                    for event in book.events() {
                        if let Err(e) =
                            send_event_marketdata(
                                &mut mkt_sender,
                                event,
                                symbol_id,
                                ts_ns,
                            ) {
                            warn!("cmp send event to marketdata failed: {e}");
                        }
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
                    &order_index,
                    symbol_id,
                    req.user_id,
                    req.order_id_hi,
                    req.order_id_lo,
                );
                update_order_index(
                    book.events(),
                    &mut order_index,
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
                            if let Err(e) = rt.block_on(write_applied_config(
                                client,
                                symbol_id,
                                &cfg,
                                ts,
                            )) {
                                warn!("matching: write_applied_config failed: {e}");
                            }
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

        if let Err(e) = flush_if_due(
            &mut wal_writer, &mut last_flush,
        ) {
            warn!("matching: wal flush_if_due failed: {e}");
        }

        // Save snapshot every 10s
        if last_snapshot.elapsed().as_secs() >= 10 {
            if let Err(e) = save_snapshot(
                &book,
                &wal_dir,
                symbol_id,
                wal_writer.last_seq(),
            ) {
                warn!("snapshot save: {}", e);
            }
            last_snapshot = Instant::now();
        }

        if let Err(e) = cmp_sender.tick() {
            warn!("cmp_sender tick (heartbeat) failed: {e}");
        }
        if let Err(e) = mkt_sender.tick() {
            warn!("mkt_sender tick (heartbeat) failed: {e}");
        }
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
            taker_ts_ns,
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
                taker_ts_ns,
            };
            // SAFETY: send() returns Ok(false) on
            // flow-control stall; receivers recover via
            // NAK / TCP replay so the bool is discarded
            // by design. Errors still propagate.
            sender.send(&mut record)?;
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
            // SAFETY: send() returns Ok(false) on
            // flow-control stall; receivers recover via
            // NAK / TCP replay so the bool is discarded
            // by design. Errors still propagate.
            sender.send(&mut record)?;
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
            // SAFETY: send() returns Ok(false) on
            // flow-control stall; receivers recover via
            // NAK / TCP replay so the bool is discarded
            // by design. Errors still propagate.
            sender.send(&mut record)?;
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
            // SAFETY: send() returns Ok(false) on
            // flow-control stall; receivers recover via
            // NAK / TCP replay so the bool is discarded
            // by design. Errors still propagate.
            sender.send(&mut record)?;
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
            // SAFETY: send() returns Ok(false) on
            // flow-control stall; receivers recover via
            // NAK / TCP replay so the bool is discarded
            // by design. Errors still propagate.
            sender.send(&mut record)?;
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
    wal.append(&mut record)
        .expect("wal append failed (config_applied) — violates 6-consistency.md invariant 7; CONFIG_APPLIED must precede CMP fan-out");
    if let Err(e) = risk_sender.send(&mut record) {
        warn!("cmp send config_applied to risk failed: {e}");
    }
    if let Err(e) = mkt_sender.send(&mut record) {
        warn!("cmp send config_applied to marketdata failed: {e}");
    }
    info!(
        "emitted config_applied v{} for symbol {}",
        config_version, symbol_id,
    );
}

/// Cancel a resting order by order_id, emit events,
/// write WAL, and send CMP to risk + marketdata.
///
/// Looks up the slab handle in `order_index` (O(1)) instead
/// of a linear slab scan. The caller must call
/// `update_order_index` after this returns so the OrderDone
/// event removes the entry.
#[allow(clippy::too_many_arguments)]
fn process_cancel(
    book: &mut Orderbook,
    wal_writer: &mut WalWriter,
    cmp_sender: &mut CmpSender,
    mkt_sender: &mut CmpSender,
    order_index: &FxHashMap<OrderKey, u32>,
    symbol_id: u32,
    user_id: u32,
    order_id_hi: u64,
    order_id_lo: u64,
) {
    let key: OrderKey = (user_id, order_id_hi, order_id_lo);
    let found = match order_index.get(&key) {
        Some(&h) => h,
        None => {
            warn!(
                "cancel: order not found \
                 user={} id={:#x}/{:#x}",
                user_id, order_id_hi, order_id_lo,
            );
            return;
        }
    };
    // Defensive: index says the order exists; verify the
    // slab slot still matches before we cancel. This catches
    // any drift between index and slab without crashing.
    let slot_check = book.orders.get(found);
    if !slot_check.is_active()
        || slot_check.user_id != user_id
        || slot_check.order_id_hi != order_id_hi
        || slot_check.order_id_lo != order_id_lo
    {
        warn!(
            "cancel: index/slab drift detected \
             user={} id={:#x}/{:#x} handle={}",
            user_id, order_id_hi, order_id_lo, found,
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
    write_events_to_wal(
        wal_writer, book, symbol_id, ts_ns,
    )
    .expect("wal append failed (cancel path) — violates 6-consistency.md invariant 1 (event total order) and invariant 5 (ORDER_DONE commit boundary)");
    for event in book.events() {
        if let Err(e) = send_event_cmp(
            cmp_sender, event, symbol_id, ts_ns,
        ) {
            warn!("cmp send cancel-event to risk failed: {e}");
        }
    }
    for event in book.events() {
        if let Err(e) = send_event_marketdata(
            mkt_sender, event, symbol_id, ts_ns,
        ) {
            warn!("cmp send cancel-event to marketdata failed: {e}");
        }
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
