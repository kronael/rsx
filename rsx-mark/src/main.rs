use rsx_dxs::wal::WalWriter;
use rsx_mark::config::load_mark_config;
use rsx_mark::config::MarkConfig;
use rsx_types::install_panic_handler;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::info;

const FLUSH_INTERVAL: Duration =
    Duration::from_millis(10);
const SWEEP_INTERVAL: Duration =
    Duration::from_secs(1);

fn run(config: &MarkConfig) -> io::Result<()> {
    let mut wal_writer = WalWriter::new(
        config.stream_id,
        &PathBuf::from(&config.wal_dir),
        64 * 1024 * 1024,
        10 * 60 * 1_000_000_000,
    )?;

    // TODO: start source connectors on tokio runtime
    // TODO: create SPSC rings per source
    // TODO: initialize SymbolMarkState vec

    let mut last_sweep = Instant::now();
    let mut last_flush = Instant::now();

    info!("mark aggregator started");

    loop {
        // 1. Drain source rings
        // TODO: for each ring, pop and aggregate

        // 2. Staleness sweep (every 1s)
        let now = Instant::now();
        if now.duration_since(last_sweep)
            >= SWEEP_INTERVAL
        {
            // TODO: sweep_stale for each symbol
            last_sweep = now;
        }

        // 3. WAL flush (every 10ms)
        if now.duration_since(last_flush)
            >= FLUSH_INTERVAL
        {
            let _ = wal_writer.flush();
            last_flush = now;
        }

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
