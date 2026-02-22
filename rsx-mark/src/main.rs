use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::RECORD_MARK_PRICE;
use rsx_dxs::wal::WalWriter;
use rsx_dxs::DxsReplayService;
use rsx_mark::aggregator::aggregate_with_staleness;
use rsx_mark::aggregator::sweep_stale_with_staleness;
use rsx_mark::config::load_mark_config;
use rsx_mark::config::MarkConfig;
use rsx_mark::source::BinanceSource;
use rsx_mark::source::CoinbaseSource;
use rsx_mark::source::PriceSource;
use rsx_mark::types::SourcePrice;
use rsx_mark::types::SymbolMarkState;
use rsx_types::time::time_ns;
use rsx_types::install_panic_handler;
use std::io;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tracing::info;

const FLUSH_INTERVAL: Duration =
    Duration::from_millis(10);
const SWEEP_INTERVAL: Duration =
    Duration::from_secs(1);

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
            src.name,
            src.ws_url,
            src.reconnect_base_ms,
            src.reconnect_max_ms,
        );
    }
}

fn run(config: &MarkConfig) -> io::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _guard = rt.enter();

    let dxs_addr: std::net::SocketAddr = config
        .listen_addr
        .parse()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid RSX_MARK_LISTEN_ADDR",
            )
        })?;
    let wal_dir = PathBuf::from(&config.wal_dir);
    let service = DxsReplayService::new(wal_dir.clone(), None)?;
    rt.spawn(async move {
        if let Err(e) = service.serve(dxs_addr).await {
            tracing::error!("dxs server error: {e}");
        }
    });

    let mut wal_writer = WalWriter::new(
        config.stream_id,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )?;

    let mark_dest: std::net::SocketAddr = env::var(
        "RSX_RISK_MARK_CMP_ADDR",
    )
    .unwrap_or_else(|_| "127.0.0.1:9105".into())
    .parse()
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid RSX_RISK_MARK_CMP_ADDR",
        )
    })?;
    let mut mark_sender = CmpSender::new(
        mark_dest,
        config.stream_id,
        &wal_dir,
    )?;

    let symbol_map = Arc::new(config.symbol_map.clone());
    let max_symbol = symbol_map
        .values()
        .copied()
        .max()
        .unwrap_or(0) as usize;
    let mut states: Vec<SymbolMarkState> =
        (0..max_symbol + 1)
            .map(|_| SymbolMarkState::new())
            .collect();

    let mut consumers = Vec::new();
    for (idx, source) in config.sources.iter().enumerate()
    {
        let (prod, cons) =
            rtrb::RingBuffer::<SourcePrice>::new(1024);
        consumers.push(cons);
        match source.name.as_str() {
            "binance" => {
                let src = BinanceSource {
                    source_id: idx as u8,
                    ws_url: source.ws_url.clone(),
                    symbol_map: symbol_map.clone(),
                    reconnect_base_ms: source
                        .reconnect_base_ms,
                    reconnect_max_ms: source
                        .reconnect_max_ms,
                    price_scale: config.price_scale,
                };
                src.start(prod);
            }
            "coinbase" => {
                let src = CoinbaseSource {
                    source_id: idx as u8,
                    ws_url: source.ws_url.clone(),
                    symbol_map: symbol_map.clone(),
                    reconnect_base_ms: source
                        .reconnect_base_ms,
                    reconnect_max_ms: source
                        .reconnect_max_ms,
                    price_scale: config.price_scale,
                };
                src.start(prod);
            }
            _ => {}
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
                if now_ns.saturating_sub(update.timestamp_ns)
                    >= config.staleness_ns
                {
                    continue;
                }
                if let Some(mut evt) =
                    aggregate_with_staleness(
                        &mut states[sid],
                        update,
                        now_ns,
                        update.symbol_id,
                        config.staleness_ns,
                    )
                {
                    if wal_writer.append(&mut evt).is_ok() {
                        let bytes = unsafe {
                            std::slice::from_raw_parts(
                                &evt as *const _ as *const u8,
                                std::mem::size_of_val(&evt),
                            )
                        };
                        let _ = mark_sender.send_raw(
                            RECORD_MARK_PRICE,
                            bytes,
                        );
                    }
                }
            }
        }

        // 2. Staleness sweep (every 1s)
        let now = Instant::now();
        if now.duration_since(last_sweep)
            >= SWEEP_INTERVAL
        {
            let now_ns = time_ns();
            for (sid, state) in
                states.iter_mut().enumerate()
            {
                if let Some(mut evt) =
                    sweep_stale_with_staleness(
                        state,
                        now_ns,
                        sid as u32,
                        config.staleness_ns,
                    )
                {
                    if wal_writer.append(&mut evt).is_ok() {
                        let bytes = unsafe {
                            std::slice::from_raw_parts(
                                &evt as *const _ as *const u8,
                                std::mem::size_of_val(&evt),
                            )
                        };
                        let _ = mark_sender.send_raw(
                            RECORD_MARK_PRICE,
                            bytes,
                        );
                    }
                }
            }
            last_sweep = now;
        }

        // 3. WAL flush (every 10ms)
        if now.duration_since(last_flush)
            >= FLUSH_INTERVAL
        {
            let _ = wal_writer.flush();
            last_flush = now;
        }

        let _ = mark_sender.tick();
        mark_sender.recv_control();

        // bare busy-spin: no yield, dedicated core
    }
}

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = match load_mark_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        "mark aggregator starting, listen={}",
        config.listen_addr
    );
    log_effective_mark_config(&config);

    loop {
        match run(&config) {
            Ok(()) => break,
            Err(e) => {
                tracing::error!(
                    "crashed: {e}, restarting in 5s"
                );
                std::thread::sleep(
                    Duration::from_secs(5),
                );
            }
        }
    }
}
