use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_dxs::wal::WalWriter;
use rsx_matching::fanout::drain_and_fanout;
use rsx_matching::wal_integration::flush_if_due;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_matching::wire::EventMessage;
use rsx_matching::wire::OrderMessage;
use rsx_types::SymbolConfig;
use std::env;
use std::io;
use std::path::PathBuf;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
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
    std::panic::set_hook(Box::new(|info| {
        eprintln!("fatal: {}", info);
        std::process::exit(1);
    }));

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

    // SPSC rings
    let (_ingress_prod, mut ingress_cons) =
        rtrb::RingBuffer::<OrderMessage>::new(2048);
    let (mut risk_prod, _risk_cons) =
        rtrb::RingBuffer::<EventMessage>::new(4096);
    let (mut gw_prod, _gw_cons) =
        rtrb::RingBuffer::<EventMessage>::new(4096);
    let (mut mkt_prod, _mkt_cons) =
        rtrb::RingBuffer::<EventMessage>::new(8192);

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

    info!("matching engine started");

    loop {
        if let Ok(order_msg) = ingress_cons.pop() {
            let mut incoming = order_msg.to_incoming();
            process_new_order(&mut book, &mut incoming);
            drain_and_fanout(
                &book,
                &mut risk_prod,
                &mut gw_prod,
                &mut mkt_prod,
            );
            let _ = write_events_to_wal(
                &mut wal_writer,
                &book,
                symbol_id,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64,
            );
            let _ = flush_if_due(
                &mut wal_writer,
                &mut last_flush,
            );
        } else if book.is_migrating() {
            book.migrate_batch(100);
        }
        // bare busy-spin: no yield, dedicated core
    }
}
