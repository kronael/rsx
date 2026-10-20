use crate::cancel::process_cancel;
use crate::order::process_order;
use rsx_book::book::Orderbook;
use rsx_book::snapshot;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecvWith;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::WalWriter;
use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_health::QueueGauge;
use rsx_matching::config::apply_scheduled_configs;
use rsx_matching::config::emit_config_applied;
use rsx_matching::config::load_applied_config;
use rsx_matching::config::load_config_from_env;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::flush_if_due;
use rsx_matching::wal::load_checkpoint;
use rsx_matching::wal::rebuild_dedup_window;
use rsx_matching::wal::rebuild_order_index_from_book;
use rsx_matching::wal::replay_wal_after_snapshot;
use rsx_matching::wal::update_order_index;
use rsx_matching::wal::CheckpointWriter;
use rsx_matching::wal::MatchingCheckpointV1;
use rsx_matching::wal::OrderKey;
use rsx_messages::CancelRequest;
use rsx_messages::OrderMessage;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::install_panic_handler;
use rsx_types::install_shutdown_handler;
use rsx_types::time_utils::time_ns;
use rsx_types::SymbolConfig;
use rustc_hash::FxHashMap;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tokio_postgres::NoTls;
use tracing::error;
use tracing::info;
use tracing::warn;

mod cancel;
mod order;

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

    // Recovery order:
    //   1. Load snapshot.bin with its embedded WAL replay boundary.
    //   2. Replay WAL records after that boundary into the book by
    //      re-executing OrderAccepted via process_new_order;
    //      this regenerates fills + emitted events deterministically.
    //   3. Bump wal_writer.next_seq so subsequent live writes
    //      don't collide with replayed seqs already on disk.
    let checkpoint = load_checkpoint(&wal_dir, symbol_id);
    let replay_from = if let Some((loaded, wal_seq)) = checkpoint {
        book = *loaded;
        // Rebuild the derived order index before applying replay deltas.
        // Dedup is rebuilt separately from the retained WAL.
        rebuild_order_index_from_book(&book, &mut order_index);
        info!(
            "book restored from checkpoint: book.seq={} wal_seq={}",
            book.sequence, wal_seq,
        );
        Some(wal_seq + 1)
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
                // what the snapshot already covers (invariant #5).
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
    // is 1 h — so seed every RECORD_ORDER_ACCEPTED still inside the
    // window, including pre- and post-snapshot records, with its
    // remaining TTL to preserve exactly-one completion after recovery.
    match rebuild_dedup_window(&mut dedup, &wal_dir, symbol_id, time_ns()) {
        Ok(n) => info!("dedup window rebuilt from wal: {n} keys"),
        Err(e) => warn!("dedup window rebuild failed: {e} — reduced dedup coverage"),
    }

    let mut last_flush = Instant::now();
    let mut last_snapshot = Instant::now();
    let mut last_config_poll = Instant::now();
    let mut last_user_maintenance = Instant::now();

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
        &initial_config,
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

    // SIGTERM/SIGINT flip this flag; the loop drains the WAL before exit.
    let shutdown = install_shutdown_handler();
    let checkpoint_writer =
        CheckpointWriter::start(&wal_dir, symbol_id).expect("failed to start checkpoint writer");

    info!("matching engine started");

    loop {
        if spin_count.is_multiple_of(CLOCK_REFRESH_SPINS) {
            clock = Instant::now();
        }
        spin_count = spin_count.wrapping_add(1);
        dedup.refresh_clock(clock);

        if shutdown.load(Ordering::SeqCst) {
            info!("shutdown signal received, draining wal");
            if let Err(e) = wal_writer.flush() {
                error!("shutdown wal flush failed: {e}");
            }
            let final_checkpoint = match snapshot::capture(&book) {
                Ok(book_snapshot) => Some(MatchingCheckpointV1::new(
                    wal_writer.last_seq(),
                    book_snapshot,
                )),
                Err(error) => {
                    warn!("shutdown snapshot capture failed: {error}");
                    None
                }
            };
            if let Err(error) = checkpoint_writer.shutdown(final_checkpoint) {
                warn!("shutdown checkpoint writer failed: {error}");
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
                    process_order(
                        &mut book,
                        &mut wal_writer,
                        &mut cast_sender,
                        &mut mkt_sender,
                        &mut order_index,
                        &mut dedup,
                        &gauges,
                        symbol_id,
                        &order_msg,
                    );
                }
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
                    update_order_index(book.events(), &mut order_index);
                }
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
        if !matches!(recv, CastRecvWith::Data)
            && clock.duration_since(last_user_maintenance).as_millis() >= 1
        {
            book.maintain_users(time_ns(), false);
            last_user_maintenance = clock;
        }

        dedup.maybe_cleanup();
        gauges
            .dedup_map_size
            .store(dedup.len() as u64, Ordering::Relaxed);

        // Poll config every 10 minutes
        if clock.duration_since(last_config_poll).as_secs() >= 600 {
            if let Some(ref client) = pg_client {
                apply_scheduled_configs(
                    &rt,
                    client,
                    &mut book,
                    &mut wal_writer,
                    &mut cast_sender,
                    &mut mkt_sender,
                    symbol_id,
                    &mut config_version,
                );
            }
            last_config_poll = clock;
        }

        if let Err(e) = flush_if_due(&mut wal_writer, &mut last_flush, clock) {
            warn!("matching: wal flush_if_due failed: {e}");
        }

        // Capture briefly on the ME thread; disk work stays off-thread.
        if clock.duration_since(last_snapshot).as_secs() >= 10 {
            match snapshot::capture(&book) {
                Ok(book_snapshot) => {
                    let checkpoint =
                        MatchingCheckpointV1::new(wal_writer.last_seq(), book_snapshot);
                    if let Err(error) = checkpoint_writer.submit(checkpoint) {
                        warn!("checkpoint submit failed: {error}");
                    }
                }
                Err(error) => {
                    warn!("checkpoint capture failed: {error}");
                }
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
