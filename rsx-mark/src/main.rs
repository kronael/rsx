use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalWriter;
use rsx_cast::ReplicationService;
use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_mark::aggregator::aggregate_with_staleness;
use rsx_mark::aggregator::sweep_stale_with_staleness;
use rsx_mark::config::load_mark_config;
use rsx_mark::config::MarkConfig;
use rsx_mark::source::BinanceSource;
use rsx_mark::source::CoinbaseSource;
use rsx_mark::source::PriceSource;
use rsx_mark::types::SourcePrice;
use rsx_mark::types::SymbolMarkState;
use rsx_types::install_panic_handler;
use rsx_types::time_utils::time_ns;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

const FLUSH_INTERVAL: Duration = Duration::from_millis(10);
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

fn log_effective_mark_config(config: &MarkConfig) {
    info!(
        "mark effective config: listen_addr={} wal_dir={} stream_id={} staleness_ns={} price_scale={} symbol_count={} source_count={}",
        config.listen_addr,
        config.wal_dir,
        config.stream_id,
        config.staleness_ns,
        config.price_scale,
        config.symbol_map.len(),
        config.sources.len(),
    );
    for (sym, sid) in &config.symbol_map {
        info!("mark symbol_map {}={}", sym, sid);
    }
    for src in &config.sources {
        info!(
            "mark source name={} ws_url={} reconnect_base_ms={} reconnect_max_ms={}",
            src.name, src.ws_url, src.reconnect_base_ms, src.reconnect_max_ms,
        );
    }
}

fn run(config: &MarkConfig) -> io::Result<()> {
    // Mark busy-spins (see main loop below) and MUST own a
    // dedicated core. Unpinned, it floats onto a hot-path
    // core (gateway/risk/ME) and starves it — the starved
    // consumer then can't drain its UDP socket, the kernel
    // RcvbufErrors, and packets drop. Pin it.
    if let Ok(core_str) = std::env::var("RSX_MARK_CORE_ID") {
        if let Ok(core_id) = core_str.parse::<usize>() {
            let setup = rsx_types::cpu::setup_hot_thread(core_id);
            tracing::info!("mark {}", setup);
            if setup.isolated == Some(false) {
                tracing::warn!("mark core {} not isolated — expect tail spikes", core_id);
            }
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = rt.enter();

    let dxs_addr: std::net::SocketAddr = config
        .listen_addr
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid RSX_MARK_LISTEN_ADDR"))?;
    let wal_dir = PathBuf::from(&config.wal_dir);
    let service = ReplicationService::new(wal_dir.clone(), rsx_cast::TlsConfig::from_env()?)?;
    rt.spawn(async move {
        if let Err(e) = service.serve(dxs_addr).await {
            tracing::error!("dxs server error: {e}");
        }
    });

    // Hot retention is 4 h. Archive handles long-term;
    // see rsx-matching/src/main.rs for the rationale.
    let mut wal_writer = WalWriter::new(config.stream_id, &wal_dir, 64 * 1024 * 1024)?;

    let mark_dest: std::net::SocketAddr = env::var("RSX_RISK_MARK_CAST_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9105".into())
        .parse()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid RSX_RISK_MARK_CAST_ADDR",
            )
        })?;
    let mut mark_sender = CastSender::new(mark_dest, config.stream_id, &wal_dir)?;

    let symbol_map = Arc::new(config.symbol_map.clone());
    let max_symbol = symbol_map.values().copied().max().unwrap_or(0) as usize;
    let mut states: Vec<SymbolMarkState> = (0..max_symbol + 1)
        .map(|_| SymbolMarkState::new())
        .collect();

    let mut consumers = Vec::new();
    for (idx, source) in config.sources.iter().enumerate() {
        let (prod, cons) = rtrb::RingBuffer::<SourcePrice>::new(1024);
        consumers.push(cons);
        match source.name.as_str() {
            "binance" => {
                let src = BinanceSource {
                    source_id: idx as u8,
                    ws_url: source.ws_url.clone(),
                    symbol_map: symbol_map.clone(),
                    reconnect_base_ms: source.reconnect_base_ms,
                    reconnect_max_ms: source.reconnect_max_ms,
                    price_scale: config.price_scale,
                };
                src.start(prod);
            }
            "coinbase" => {
                let src = CoinbaseSource {
                    source_id: idx as u8,
                    ws_url: source.ws_url.clone(),
                    symbol_map: symbol_map.clone(),
                    reconnect_base_ms: source.reconnect_base_ms,
                    reconnect_max_ms: source.reconnect_max_ms,
                    price_scale: config.price_scale,
                };
                src.start(prod);
            }
            _ => {}
        }
    }

    // Health server: RSX_MARK_HEALTH_ADDR=127.0.0.1:9204
    // GET /health → 200/503 liveness
    // GET /ready   → 200/503 readiness (false when all sources stale)
    // GET /metrics → JSON (publish counter, stale sources)
    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    gauges.ready.store(true, Ordering::Relaxed);
    gauges.state_idx.store(4, Ordering::Relaxed); // "running"
    if let Ok(addr_str) = env::var("RSX_MARK_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let g = gauges.clone();
            rsx_health::spawn_health_server(addr, move || {
                let publishes = g.publishes.load(Ordering::Relaxed);
                let drops = g.drops.load(Ordering::Relaxed);
                // ready=false when all sources are stale
                // (drops holds stale source count)
                let ready = g.ready.load(Ordering::Relaxed);
                HealthSnapshot {
                    live: g.live.load(Ordering::Relaxed),
                    ready,
                    saturation: 0.0,
                    queues: vec![],
                    counters: vec![
                        CounterGauge {
                            name: "publishes",
                            value: publishes,
                        },
                        CounterGauge {
                            name: "stale_sources",
                            value: drops,
                        },
                    ],
                    state: g.state_label(),
                }
            });
        } else {
            warn!("RSX_MARK_HEALTH_ADDR: invalid addr '{addr_str}'");
        }
    }

    let mut last_sweep = Instant::now();
    let mut last_flush = Instant::now();

    info!("mark aggregator started");

    loop {
        // 1. Drain source rings
        for cons in consumers.iter_mut() {
            while let Ok(update) = cons.pop() {
                let sid = update.symbol_id as usize;
                if sid >= states.len() {
                    continue;
                }
                let now_ns = time_ns();
                // Skip stale updates before aggregate phase
                if now_ns.saturating_sub(update.timestamp_ns) >= config.staleness_ns {
                    continue;
                }
                if let Some(mut evt) = aggregate_with_staleness(
                    &mut states[sid],
                    update,
                    now_ns,
                    update.symbol_id,
                    config.staleness_ns,
                ) {
                    if let Ok(framed) = wal_writer.prepare(&mut evt) {
                        if wal_writer.append_framed(&framed).is_ok() {
                            if let Err(e) = mark_sender.send_framed(&framed) {
                                warn!("mark: cmp send (aggregate) failed: {e}");
                            } else {
                                gauges.publishes.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        }

        // 2. Staleness sweep (every 1s)
        let now = Instant::now();
        if now.duration_since(last_sweep) >= SWEEP_INTERVAL {
            let now_ns = time_ns();
            let mut stale_count: u64 = 0;
            for (sid, state) in states.iter_mut().enumerate() {
                if let Some(mut evt) =
                    sweep_stale_with_staleness(state, now_ns, sid as u32, config.staleness_ns)
                {
                    stale_count += 1;
                    if let Ok(framed) = wal_writer.prepare(&mut evt) {
                        if wal_writer.append_framed(&framed).is_ok() {
                            if let Err(e) = mark_sender.send_framed(&framed) {
                                warn!("mark: cmp send (sweep) failed: {e}");
                            }
                        }
                    }
                }
            }
            // ready=false when ALL symbols are stale. stale_count
            // == states.len() means no live price for any symbol.
            let all_stale = !states.is_empty() && stale_count == states.len() as u64;
            gauges.ready.store(!all_stale, Ordering::Relaxed);
            gauges.drops.store(stale_count, Ordering::Relaxed);
            last_sweep = now;
        }

        // 3. WAL flush (every 10ms)
        if now.duration_since(last_flush) >= FLUSH_INTERVAL {
            if let Err(e) = wal_writer.flush() {
                warn!("mark: wal flush failed: {e}");
            }
            last_flush = now;
        }

        if let Err(e) = mark_sender.tick() {
            warn!("mark: cast_sender tick failed: {e}");
        }
        mark_sender.recv_control();

        // Off the critical path: mark prices tick on external-feed
        // cadence (~10/s/symbol) and feed margin/liquidation, which
        // tolerate second-scale latency. A 250µs poll drains the
        // input ring promptly without burning a core — ergonomic,
        // like the other off-path services. (Was a dedicated-core
        // busy-spin; that starved hot-path cores when unpinned.)
        std::thread::sleep(Duration::from_micros(250));
    }
}

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    // Rustls 0.23 requires an explicit default CryptoProvider when
    // multiple are compiled in (we get both ring + aws-lc-rs via the
    // tokio-tungstenite rustls-tls-native-roots feature + rsx-cast).
    // Without this, the first TLS handshake panics. Install once,
    // ignore the duplicate-install Err (returned when already set,
    // e.g. when a test harness ran first).
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = match load_mark_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {}", e);
            std::process::exit(1);
        }
    };

    info!("mark aggregator starting, listen={}", config.listen_addr);
    log_effective_mark_config(&config);

    loop {
        match run(&config) {
            Ok(()) => break,
            Err(e) => {
                tracing::error!("crashed: {e}, restarting in 5s");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}
