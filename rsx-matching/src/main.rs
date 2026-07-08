use rsx_book::book::Orderbook;
use rsx_book::event::CANCEL_USER;
use rsx_book::event::FAIL_DUPLICATE;
use rsx_book::event::REASON_CANCELLED;
use rsx_book::matching::process_new_order;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::WalWriter;
use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_health::QueueGauge;
use rsx_matching::config::load_applied_config;
use rsx_matching::config::poll_scheduled_configs;
use rsx_matching::config::write_applied_config;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::flush_if_due;
use rsx_matching::wal::load_snapshot;
use rsx_matching::wal::load_wal_seq;
use rsx_matching::wal::publish_events;
use rsx_matching::wal::rebuild_dedup_window;
use rsx_matching::wal::replay_wal_after_snapshot;
use rsx_matching::wal::save_snapshot;
use rsx_matching::wire::to_incoming;
use rsx_messages::CancelRequest;
use rsx_messages::ConfigAppliedRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderMessage;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::install_panic_handler;
use rsx_types::time_utils::time_ms;
use rsx_types::time_utils::time_ns;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::SymbolConfig;
use rsx_types::NONE;
use rustc_hash::FxHashMap;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tokio_postgres::NoTls;
use tracing::error;
use tracing::info;
use tracing::warn;

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
fn update_order_index(events: &[rsx_book::event::Event], index: &mut FxHashMap<OrderKey, u32>) {
    for event in events {
        match *event {
            rsx_book::event::Event::OrderInserted {
                handle,
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.insert((user_id, order_id_hi, order_id_lo), handle);
            }
            rsx_book::event::Event::OrderDone {
                user_id,
                order_id_hi,
                order_id_lo,
                ..
            } => {
                index.remove(&(user_id, order_id_hi, order_id_lo));
            }
            _ => {}
        }
    }
}

/// Rebuild `order_index` from a freshly restored book's
/// active resting orders. A snapshot persists the book's slab
/// but not the (user,oid)->handle map, so without this a
/// post-restart cancel for a pre-snapshot resting order would
/// miss the index and silently no-op. Scans every slab handle
/// and re-keys the active ones — same (user,oid)->handle shape
/// `update_order_index` builds from OrderInserted events.
/// (bugs.md ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD, index half)
fn rebuild_order_index_from_book(
    book: &rsx_book::book::Orderbook,
    index: &mut FxHashMap<OrderKey, u32>,
) {
    index.clear();
    for handle in 0..book.orders.len() {
        let slot = book.orders.get(handle);
        if !slot.is_active() {
            continue;
        }
        index.insert((slot.user_id, slot.order_id_hi, slot.order_id_lo), handle);
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
        "matching effective config: symbol_id={} tick_size={} lot_size={} price_decimals={} qty_decimals={} db_enabled={} wal_dir={} me_cast_addr={} risk_cast_addr={} md_cast_addr={}",
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
    let price_decimals = get_env_u8("RSX_ME_PRICE_DECIMALS")?;
    let qty_decimals = get_env_u8("RSX_ME_QTY_DECIMALS")?;
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

    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

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
            let setup = rsx_types::cpu::setup_hot_thread(core_id);
            info!("me {}", setup);
            if setup.isolated == Some(false) {
                tracing::warn!("me core {} not isolated — expect tail spikes", core_id);
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
                .map_err(|e| io::Error::other(format!("db connect: {}", e)))?;
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
                        info!("no applied config found, using env config");
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
    let wal_dir = env::var("RSX_ME_WAL_DIR").unwrap_or_else(|_| "./tmp/wal".to_string());
    let mut wal_writer = WalWriter::new(symbol_id, &PathBuf::from(&wal_dir), 64 * 1024 * 1024)
        // SAFETY: fail-fast at startup
        .expect("failed to create wal writer");

    let mut dedup = DedupTracker::new();
    let mut order_index: FxHashMap<OrderKey, u32> = FxHashMap::default();

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
    let snapshot_loaded = load_snapshot(&wal_dir, symbol_id);
    let replay_from = if let Some(loaded) = snapshot_loaded {
        book = *loaded;
        // The snapshot persists the book slab but not the
        // (user,oid)->handle index — rebuild it from the
        // restored resting orders so post-restart cancels for
        // pre-snapshot orders hit. WAL replay below layers its
        // own OrderInserted/OrderDone deltas on top. Dedup is
        // rebuilt separately from the WAL after replay (see
        // rebuild_dedup_window below).
        // (bugs.md ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD, index half)
        rebuild_order_index_from_book(&book, &mut order_index);
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
        info!("no snapshot found — replaying WAL from seq 1");
        Some(1)
    };
    if let Some(start_seq) = replay_from {
        match replay_wal_after_snapshot(&mut book, &mut order_index, &wal_dir, symbol_id, start_seq)
        {
            Ok(last_seq) if last_seq >= start_seq => {
                wal_writer.set_next_seq(last_seq + 1);
            }
            Ok(_) => {
                // No records replayed past start_seq, but the
                // snapshot already covers seqs up to start_seq-1
                // on disk. The fresh writer's next_seq defaults
                // to 1, which would reuse/regress WAL seqs the
                // snapshot implies — violating invariant #5
                // (tips monotonic). Advance to the snapshot tip
                // so the next live append never regresses below
                // what the snapshot already covers.
                // (bugs.md ME-NEXT-SEQ-REGRESSION)
                wal_writer.set_next_seq(start_seq.max(1));
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

    // Rebuild the dedup window from the WAL. The book snapshot does not
    // persist the dedup set and only covers ~10 s, while the dedup window
    // is 300 s — so seed every RECORD_ORDER_ACCEPTED still inside the
    // window (pre- AND post-snapshot) with its remaining TTL. Without this,
    // a client resend of a pre-snapshot order after a crash would
    // re-execute (violates exactly-one-completion).
    // (bugs.md ME-SNAPSHOT-NO-INDEX-DEDUP-REBUILD, dedup half)
    match rebuild_dedup_window(&mut dedup, &wal_dir, symbol_id, time_ns()) {
        Ok(n) => info!("dedup window rebuilt from wal: {n} keys"),
        Err(e) => warn!("dedup window rebuild failed: {e} — reduced dedup coverage"),
    }

    let mut last_flush = Instant::now();
    let mut last_snapshot = Instant::now();
    let mut last_config_poll = Instant::now();

    // Cached monotonic clock: sampled once every CLOCK_REFRESH_SPINS loop
    // iterations and reused for dedup pruning + the flush/snapshot/config
    // timers, so the hot loop stops paying ~4 `Instant::now()` per spin.
    // The hour-scale dedup window and 10 ms flush tolerate the coarse tick.
    const CLOCK_REFRESH_SPINS: u32 = 1024;
    let mut clock = Instant::now();
    let mut spin_count: u32 = 0;

    // casting/UDP: receive orders from Risk
    let me_addr: SocketAddr = env::var("RSX_ME_CAST_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9100".into())
        .parse()
        // SAFETY: fail-fast at startup
        .expect("invalid RSX_ME_CAST_ADDR");
    // NAK destination: risk's ME sender bind addr
    // (RSX_RISK_ME_SEND_ADDR), with RSX_RISK_CAST_ADDR as fallback.
    let risk_nak_addr: SocketAddr = env::var("RSX_RISK_ME_SEND_ADDR")
        .or_else(|_| env::var("RSX_RISK_CAST_ADDR"))
        .unwrap_or_else(|_| "127.0.0.1:9101".into())
        .parse()
        // SAFETY: fail-fast at startup
        .expect("invalid NAK sender addr");
    // Risk's dedicated port for ME events (fills, BBO, etc.)
    let risk_me_recv_addr: SocketAddr = env::var("RSX_RISK_ME_RECV_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:28301".into())
        .parse()
        // SAFETY: fail-fast at startup
        .expect("invalid RSX_RISK_ME_RECV_ADDR");

    let mut cast_receiver = CastReceiver::new(me_addr, risk_nak_addr)
        // SAFETY: fail-fast at startup
        .expect("failed to bind cast receiver");

    let mut cast_sender = CastSender::new(risk_me_recv_addr, symbol_id, &PathBuf::from(&wal_dir))
        // SAFETY: fail-fast at startup
        .expect("failed to create cast sender");

    // casting/UDP: send events to Marketdata
    let mkt_addr: SocketAddr = env::var("RSX_MD_CAST_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9103".into())
        .parse()
        // SAFETY: fail-fast at startup
        .expect("invalid RSX_MD_CAST_ADDR");
    let mut mkt_sender = CastSender::new(mkt_addr, symbol_id, &PathBuf::from(&wal_dir))
        // SAFETY: fail-fast at startup
        .expect("failed to create MD cast sender");
    log_effective_matching_config(
        &book.config,
        &db_url,
        &wal_dir,
        &me_addr,
        &risk_nak_addr,
        &mkt_addr,
    );

    // DXS sidecar
    if let Ok(dxs_addr) = env::var("RSX_ME_REPLICATION_BIND_ADDR") {
        let addr: std::net::SocketAddr = dxs_addr
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_ME_REPLICATION_BIND_ADDR");
        let wal_path = PathBuf::from(&wal_dir);
        // Replication is TLS-mandatory; read certs before the
        // thread so a missing cert fails fast at startup.
        let tls = rsx_cast::TlsConfig::from_env()
            // SAFETY: fail-fast at startup
            .expect(
                "replication requires TLS \
                 (run scripts/gen-snakeoil-certs.sh)",
            );
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                // SAFETY: fail-fast at startup
                .expect("tokio runtime for dxs");
            let service = rsx_cast::ReplicationService::new(wal_path, tls)
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
        info!("dxs sidecar spawned on {}", dxs_addr);
    }

    // Emit CONFIG_APPLIED for this symbol on startup (if we have a version)
    if config_version > 0 {
        emit_config_applied(
            &mut wal_writer,
            &mut cast_sender,
            &mut mkt_sender,
            symbol_id,
            config_version,
            0,
        );
    }

    // Health server: RSX_ME_HEALTH_ADDR=127.0.0.1:9202
    // GET /health → 200/503 liveness
    // GET /ready   → 200/503 readiness
    // GET /metrics → JSON (match throughput, dedup map size)
    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    gauges.ready.store(true, Ordering::Relaxed);
    gauges.state_idx.store(4, Ordering::Relaxed); // "running"
    if let Ok(addr_str) = env::var("RSX_ME_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let g = gauges.clone();
            rsx_health::spawn_health_server(addr, move || HealthSnapshot {
                live: g.live.load(Ordering::Relaxed),
                ready: g.ready.load(Ordering::Relaxed),
                saturation: 0.0,
                queues: vec![QueueGauge {
                    name: "dedup_map",
                    used: g.dedup_map_size.load(Ordering::Relaxed),
                    cap: 65536,
                }],
                counters: vec![
                    CounterGauge {
                        name: "orders_processed",
                        value: g.orders_processed.load(Ordering::Relaxed),
                    },
                    CounterGauge {
                        name: "publishes",
                        value: g.publishes.load(Ordering::Relaxed),
                    },
                ],
                state: g.state_label(),
            });
        } else {
            warn!("RSX_ME_HEALTH_ADDR: invalid addr '{addr_str}'");
        }
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
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }

    info!("matching engine started");

    loop {
        if spin_count.is_multiple_of(CLOCK_REFRESH_SPINS) {
            clock = Instant::now();
        }
        spin_count = spin_count.wrapping_add(1);
        dedup.refresh_clock(clock);

        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("shutdown signal received, draining wal");
            if let Err(e) = wal_writer.flush() {
                error!("shutdown wal flush failed: {e}");
            }
            if let Err(e) = save_snapshot(&book, &wal_dir, symbol_id, wal_writer.last_seq()) {
                warn!("shutdown snapshot save failed: {e}");
            }
            info!("matching engine shutdown complete");
            std::process::exit(0);
        }
        // Receive orders/cancels via casting/UDP from Risk.
        // Zero-copy: the order-processing body runs inside the
        // callback, borrowing the receiver's recv buffer — no
        // per-message Vec allocation. The closure cannot
        // `continue`/`break` the outer loop, but the Data body
        // never needs to (the dup path is an if/else, not an
        // early return); Faulted/Empty/Reconnect are handled in
        // the match below where `continue` is legal.
        let recv = cast_receiver.try_recv_with(|hdr, payload| {
            if hdr.record_type == RECORD_ORDER_REQUEST {
            if let Some(order_msg) = decode_payload::<OrderMessage>(payload) {
                // F4.3 — per-stage latency trace. Stage
                // `me_in` = order arrived at matching engine.
                // t_us measured against gateway submit ts.
                rsx_log::latency_sample!(
                    "me_in",
                    order_msg.order_id_hi,
                    order_msg.order_id_lo,
                    order_msg.timestamp_ns
                );
                // Dedup check
                let is_dup = dedup.check_and_insert(
                    order_msg.user_id,
                    order_msg.order_id_hi,
                    order_msg.order_id_lo,
                );
                // Sub-stage: dedup completed (after FxHashMap
                // insert). Anchored on the same t0 as me_in.
                rsx_log::latency_sample!(
                    "me_dedup_done",
                    order_msg.order_id_hi,
                    order_msg.order_id_lo,
                    order_msg.timestamp_ns
                );

                // Update order counter regardless of dup status.
                gauges.orders_processed.fetch_add(
                    1, Ordering::Relaxed,
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
                        reason: FAIL_DUPLICATE,
                        _pad: [0; 23],
                    };
                    {
                        let framed = wal_writer
                            .prepare(&mut fail)
                            .expect("wal prepare failed (duplicate-reject)");
                        wal_writer
                            .append_framed(&framed)
                            .expect("wal append failed (duplicate-reject) — violates 6-consistency.md invariant 7 (matching engine persists orderbook via snapshot + WAL)");
                        // SEQ-1: send to BOTH streams. Any WAL'd
                        // record skipped on a stream is a seq hole
                        // there → false FAULTED. marketdata ignores
                        // the type.
                        if let Err(e) =
                            cast_sender.send_framed(&framed)
                        {
                            warn!(
                                "cmp send fail-record \
                                 (duplicate): {e}"
                            );
                        }
                        if let Err(e) =
                            mkt_sender.send_framed(&framed)
                        {
                            warn!(
                                "mkt send fail-record \
                                 (duplicate): {e}"
                            );
                        }
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
                    {
                        let framed = wal_writer
                            .prepare(&mut accepted)
                            .expect("wal prepare failed (order-accepted)");
                        wal_writer
                            .append_framed(&framed)
                            .expect("wal append failed (order-accepted) — violates 6-consistency.md invariant 7 (WAL persistence) and breaks dedup on replay");
                        // SEQ-1: OrderAccepted was WAL-only, but it
                        // consumes a WAL seq → a hole on BOTH live
                        // streams every accepted order (the ~2/sec
                        // FAULT source). Fan out to both; consumers
                        // ignore RECORD_ORDER_ACCEPTED (dedup record).
                        if let Err(e) =
                            cast_sender.send_framed(&framed)
                        {
                            warn!("cmp send order-accepted: {e}");
                        }
                        if let Err(e) =
                            mkt_sender.send_framed(&framed)
                        {
                            warn!("mkt send order-accepted: {e}");
                        }
                    }
                    // Sub-stage: OrderAcceptedRecord appended.
                    rsx_log::latency_sample!(
                        "me_wal_accepted_done",
                        order_msg.order_id_hi,
                        order_msg.order_id_lo,
                        order_msg.timestamp_ns
                    );

                    let mut incoming = to_incoming(&order_msg);
                    process_new_order(
                        &mut book, &mut incoming,
                    );
                    // Sub-stage: match cycle finished.
                    rsx_log::latency_sample!(
                        "me_match_done",
                        order_msg.order_id_hi,
                        order_msg.order_id_lo,
                        order_msg.timestamp_ns
                    );

                    // Publish events: WAL append + cast send to
                    // risk + (selective) cast send to marketdata,
                    // each event prepared once (one CRC). Crash
                    // on WAL failure rather than lose fills.
                    let ts_ns = time_ns();
                    publish_events(
                        &mut wal_writer,
                        &mut cast_sender,
                        &mut mkt_sender,
                        &book,
                        symbol_id,
                        ts_ns,
                    )
                    .expect("publish_events failed (event path) — violates 6-consistency.md invariant 1 (totally-ordered events) and ordering rule 'Fills precede ORDER_DONE' (§2)");
                    gauges.publishes.fetch_add(
                        1, Ordering::Relaxed,
                    );
                    // Sub-stage: events flushed + sent.
                    rsx_log::latency_sample!(
                        "me_wal_events_done",
                        order_msg.order_id_hi,
                        order_msg.order_id_lo,
                        order_msg.timestamp_ns
                    );

                    // Maintain the (user, oid) -> handle
                    // index so subsequent cancels are O(1).
                    update_order_index(
                        book.events(),
                        &mut order_index,
                    );
                    // Sub-stage: order index updated.
                    rsx_log::latency_sample!(
                        "me_index_done",
                        order_msg.order_id_hi,
                        order_msg.order_id_lo,
                        order_msg.timestamp_ns
                    );

                    // F4.3 — per-stage latency trace. Stage
                    // `me_out` = ME finished matching and is
                    // about to forward events to risk. t_us
                    // measured against the order's gateway
                    // submit timestamp. We only log once per
                    // incoming order (against its oid).
                    rsx_log::latency_sample!(
                        "me_out",
                        order_msg.order_id_hi,
                        order_msg.order_id_lo,
                        order_msg.timestamp_ns
                    );

                    // Keep the touch inside the 1:1 zone 0: recenter the
                    // compression map when the mid drifts past half of
                    // zone 0. Migration then proceeds lazily
                    // (resolve_level per touched price) and on idle
                    // cycles (migrate_batch below).
                    maybe_recenter(&mut book);
                }
            } // end if let Some(order_msg)
            } else if hdr.record_type == RECORD_CANCEL_REQUEST {
            if let Some(req) = decode_payload::<CancelRequest>(payload) {
                process_cancel(
                    &mut book,
                    &mut wal_writer,
                    &mut cast_sender,
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
            } // end if let Some(req)
            }
        });
        if let CastRecvWith::Faulted {
            last_delivered_seq,
            gap_start,
            gap_end_inclusive,
        } = recv
        {
            // Drop-safe skip. FAULTED = an unrecoverable gap in the
            // risk->ME order stream. That stream is recovered at the
            // APPLICATION layer, not the transport: a dropped pre-ack
            // order is re-sent by the client (no-ack-within-timeout,
            // spec 49-webproto) and deduped on the ME's WAL
            // (RECORD_ORDER_ACCEPTED) => exactly-once. The ME
            // re-sequences on output (its own WAL seq), so an inbound
            // gap is NOT an output gap — risk/recorder/marketdata still
            // see a contiguous ME stream. So skip the gap and resume
            // live rather than replay-or-panic. (ME's WAL replication
            // SERVER, RSX_ME_REPLICATION_BIND_ADDR, still lets RISK
            // recover FILL delivery — a different, still-required path.)
            let skipped = gap_end_inclusive.saturating_sub(gap_start) + 1;
            gauges.drops.fetch_add(skipped, Ordering::Relaxed);
            warn!(
                "matching loop FAULTED: skipping unrecoverable order \
                 gap [{}..={}] ({} seq) after last_delivered={}; \
                 clients re-send dropped pre-ack orders (WAL dedup = \
                 exactly-once)",
                gap_start, gap_end_inclusive, skipped, last_delivered_seq,
            );
            cast_receiver.reset_after_replay(gap_end_inclusive);
            continue;
        }
        // Nothing delivered this tick (Empty/Reconnect): make
        // progress on a background compression-map migration if
        // one is in flight. Data was already processed in the
        // callback above.
        if !matches!(recv, CastRecvWith::Data) && book.is_migrating() {
            book.migrate_batch(100);
        }

        dedup.maybe_cleanup();
        gauges
            .dedup_map_size
            .store(dedup.len() as u64, Ordering::Relaxed);

        // Poll config every 10 minutes
        if clock.duration_since(last_config_poll).as_secs() >= 600 {
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
                            book.update_config(cfg.config);
                            config_version = cfg.config_version;
                            let ts = time_ns();
                            if let Err(e) =
                                rt.block_on(write_applied_config(client, symbol_id, &cfg, ts))
                            {
                                warn!("matching: write_applied_config failed: {e}");
                            }
                            emit_config_applied(
                                &mut wal_writer,
                                &mut cast_sender,
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
            last_config_poll = clock;
        }

        if let Err(e) = flush_if_due(&mut wal_writer, &mut last_flush, clock) {
            warn!("matching: wal flush_if_due failed: {e}");
        }

        // Save snapshot every 10s
        if clock.duration_since(last_snapshot).as_secs() >= 10 {
            if let Err(e) = save_snapshot(&book, &wal_dir, symbol_id, wal_writer.last_seq()) {
                warn!("snapshot save: {}", e);
            }
            last_snapshot = clock;
        }

        if let Err(e) = cast_sender.tick() {
            warn!("cast_sender tick (heartbeat) failed: {e}");
        }
        if let Err(e) = mkt_sender.tick() {
            warn!("mkt_sender tick (heartbeat) failed: {e}");
        }
        cast_sender.recv_control();
        mkt_sender.recv_control();
    }
}

fn emit_config_applied(
    wal: &mut WalWriter,
    risk_sender: &mut CastSender,
    mkt_sender: &mut CastSender,
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
    let framed = wal
        .prepare(&mut record)
        .expect("wal prepare failed (config_applied)");
    wal.append_framed(&framed)
        .expect("wal append failed (config_applied) — violates 6-consistency.md invariant 7; CONFIG_APPLIED must precede cast fan-out");
    if let Err(e) = risk_sender.send_framed(&framed) {
        warn!("cmp send config_applied to risk failed: {e}");
    }
    if let Err(e) = mkt_sender.send_framed(&framed) {
        warn!("cmp send config_applied to marketdata failed: {e}");
    }
    info!(
        "emitted config_applied v{} for symbol {}",
        config_version, symbol_id,
    );
}

/// Recenter the compression map on the current mid when it has drifted
/// past half of zone 0, so the touch stays in the 1:1 zone (strict
/// price-time priority; keeps the compressed-slot slow paths cold).
/// Skips while one-sided (no mid to center on).
///
/// Uses `recenter_now` (eager: swap + full migration in one shot). Lazy
/// per-order migration is NOT correct for live matching — a marketable
/// order's crossing liquidity can sit outside the migrated band, so the
/// ME must never trade against a partially-migrated book. The swap +
/// migration is O(old book size) but recenters are rare (>2.5% drift).
fn maybe_recenter(book: &mut Orderbook) {
    if book.best_bid_tick == NONE || book.best_ask_tick == NONE {
        return;
    }
    let mid = (book.best_bid_px + book.best_ask_px) / 2;
    if book.should_recenter(mid) {
        book.recenter_now(mid);
    }
}

/// Cancel a resting order by order_id, emit events,
/// write WAL, and send cast to risk + marketdata.
///
/// Looks up the slab handle in `order_index` (O(1)) instead
/// of a linear slab scan. The caller must call
/// `update_order_index` after this returns so the OrderDone
/// event removes the entry.
#[allow(clippy::too_many_arguments)]
fn process_cancel(
    book: &mut Orderbook,
    wal_writer: &mut WalWriter,
    cast_sender: &mut CastSender,
    mkt_sender: &mut CastSender,
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
    // order_index is kept in lockstep with the slab (updated on
    // every OrderInserted/OrderDone, rebuilt on restore), so a
    // present key always points at a live slot for this
    // (user, oid). Assert that invariant in debug; trust it in
    // release rather than silently swallowing the cancel.
    let slot = book.orders.get(found);
    debug_assert!(
        slot.is_active()
            && slot.user_id == user_id
            && slot.order_id_hi == order_id_hi
            && slot.order_id_lo == order_id_lo,
        "order_index/slab drift: user={} id={:#x}/{:#x} handle={}",
        user_id,
        order_id_hi,
        order_id_lo,
        found,
    );
    let remaining_qty = slot.remaining_qty;
    let old_bid = book.best_bid_tick;
    let old_ask = book.best_ask_tick;
    let old_bid_px = book.best_bid_px;
    let old_ask_px = book.best_ask_px;

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

    // Emit BBO if best bid or ask changed (tick OR price). `current_bbo`
    // reads the maintained best px and the side-correct qty at that price
    // — a compressed best level can hold the other side / other prices, so
    // the FIFO head is not the BBO.
    if book.best_bid_tick != old_bid
        || book.best_ask_tick != old_ask
        || book.best_bid_px != old_bid_px
        || book.best_ask_px != old_ask_px
    {
        let (bid_px, bid_qty, ask_px, ask_qty) = book.current_bbo();
        book.emit(rsx_book::event::Event::BBO {
            bid_px: Price(bid_px),
            bid_qty: Qty(bid_qty),
            ask_px: Price(ask_px),
            ask_qty: Qty(ask_qty),
        });
    }

    let ts_ns = time_ns();
    publish_events(
        wal_writer, cast_sender, mkt_sender, book, symbol_id, ts_ns,
    )
    .expect("publish_events failed (cancel path) — violates 6-consistency.md invariant 1 (event total order) and invariant 5 (ORDER_DONE commit boundary)");
}
