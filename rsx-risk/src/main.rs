use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::FillRecord;
use rsx_risk::config::load_shard_config;
use rsx_risk::persist::run_persist_worker;
use rsx_risk::replay::acquire_advisory_lock;
use rsx_risk::replay::load_from_postgres;
use rsx_risk::replay::replay_from_wal;
use rsx_risk::rings::MarkPriceUpdate;
use rsx_risk::rings::ShardRings;
use rsx_risk::shard::RiskShard;
use rsx_risk::BboUpdate;
use rsx_risk::FillEvent;
use rsx_risk::OrderRequest;
use rsx_risk::OrderResponse;
use rsx_risk::PersistEvent;
use rsx_types::install_panic_handler;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tracing::error;
use tracing::info;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = load_shard_config()
        .expect("failed to load shard config");
    let shard_id = config.shard_id;
    let shard_count = config.shard_count;
    let max_symbols = config.max_symbols;

    info!(
        "risk shard {} starting ({} shards, {} symbols)",
        shard_id, shard_count, max_symbols,
    );

    loop {
        match run(shard_id, shard_count, max_symbols) {
            Ok(()) => break,
            Err(e) => {
                error!(
                    "crashed: {e}, restarting in 5s"
                );
                std::thread::sleep(
                    Duration::from_secs(5),
                );
            }
        }
    }
}

fn run(
    shard_id: u32,
    _shard_count: u32,
    max_symbols: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_shard_config()?;
    let mut shard = RiskShard::new(config);

    // Cold start from Postgres if DATABASE_URL set
    let db_url = env::var("DATABASE_URL").ok();
    if let Some(ref url) = db_url {
        let rt =
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
        rt.block_on(async {
            let (client, connection) =
                tokio_postgres::connect(
                    url,
                    tokio_postgres::NoTls,
                )
                .await?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    error!("pg connection error: {e}");
                }
            });
            acquire_advisory_lock(&client, shard_id)
                .await?;
            let state = load_from_postgres(
                &client,
                shard_id,
                shard_id, // same shard
                max_symbols,
            )
            .await?;
            shard.load_state(state);
            info!("cold start loaded from postgres");
            Ok::<(), Box<dyn std::error::Error>>(())
        })?;
    }

    // WAL replay
    let wal_dir = env::var("RSX_RISK_WAL_DIR")
        .unwrap_or_else(|_| "./tmp/wal".into());
    let symbol_ids: Vec<u32> =
        (0..max_symbols as u32).collect();
    let replayed = replay_from_wal(
        &mut shard,
        std::path::Path::new(&wal_dir),
        &symbol_ids,
    )?;
    if replayed > 0 {
        info!("replayed {} fills from wal", replayed);
    }

    // Persist worker (if DB available)
    let (persist_prod, persist_cons) =
        rtrb::RingBuffer::<PersistEvent>::new(8192);
    shard.set_persist_producer(persist_prod);

    if let Some(ref url) = db_url {
        let url = url.clone();
        let sid = shard_id;
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder
                ::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio rt");
            rt.block_on(async move {
                let (client, connection) =
                    tokio_postgres::connect(
                        &url,
                        tokio_postgres::NoTls,
                    )
                    .await
                    .expect("pg connect for persist");
                tokio::spawn(async move {
                    if let Err(e) = connection.await {
                        error!(
                            "persist pg error: {e}"
                        );
                    }
                });
                run_persist_worker(
                    persist_cons, client, sid,
                )
                .await;
            });
        });
    }

    // Pin to core if specified
    if let Ok(core_str) =
        env::var("RSX_RISK_CORE_ID")
    {
        if let Ok(core_id) =
            core_str.parse::<usize>()
        {
            let ids = core_affinity::get_core_ids()
                .unwrap_or_default();
            if let Some(id) = ids.get(core_id) {
                core_affinity::set_for_current(*id);
                info!("pinned to core {}", core_id);
            }
        }
    }

    // CMP/UDP connections
    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            .expect("invalid RSX_RISK_CMP_ADDR");
    let gw_addr: SocketAddr =
        env::var("RSX_GW_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9102".into())
            .parse()
            .expect("invalid RSX_GW_CMP_ADDR");
    let me_addr: SocketAddr =
        env::var("RSX_ME_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".into())
            .parse()
            .expect("invalid RSX_ME_CMP_ADDR");

    // Receive orders from Gateway
    let mut gw_receiver = CmpReceiver::new(
        risk_addr, gw_addr, 0,
    )
    .expect("failed to bind risk CMP receiver");

    // Receive fills from ME
    let mut me_receiver = CmpReceiver::new(
        "127.0.0.1:0".parse().unwrap(), me_addr, 0,
    )
    .expect("failed to bind ME fill receiver");

    // Send validated orders to ME
    let mut me_sender = CmpSender::new(
        me_addr,
        0,
        Path::new(&wal_dir),
    )
    .expect("failed to create ME CMP sender");

    // Send responses to Gateway
    let mut gw_sender = CmpSender::new(
        gw_addr,
        0,
        Path::new(&wal_dir),
    )
    .expect("failed to create GW CMP sender");

    // SPSC rings for run_once (internal)
    let (mut fill_prod, fill_cons) =
        rtrb::RingBuffer::<FillEvent>::new(4096);
    let (mut order_prod, order_cons) =
        rtrb::RingBuffer::<OrderRequest>::new(2048);
    let (_mark_prod, mark_cons) =
        rtrb::RingBuffer::<MarkPriceUpdate>::new(256);
    let (_bbo_prod, bbo_cons) =
        rtrb::RingBuffer::<BboUpdate>::new(256);
    let (resp_prod, mut resp_cons) =
        rtrb::RingBuffer::<OrderResponse>::new(2048);

    let mut rings = ShardRings {
        fill_consumers: vec![fill_cons],
        order_consumer: order_cons,
        mark_consumer: mark_cons,
        bbo_consumers: vec![bbo_cons],
        response_producer: resp_prod,
    };

    info!("risk shard {} running", shard_id);

    loop {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Pump CMP -> SPSC rings
        // Orders from Gateway
        if let Some((_hdr, payload)) =
            gw_receiver.try_recv()
        {
            if payload.len()
                >= std::mem::size_of::<OrderRequest>()
            {
                let order = unsafe {
                    std::ptr::read(
                        payload.as_ptr()
                            as *const OrderRequest,
                    )
                };
                let _ = order_prod.push(order);
            }
        }

        // Fills from ME
        if let Some((preamble, payload)) =
            me_receiver.try_recv()
        {
            if preamble.record_type == RECORD_FILL
                && payload.len()
                    >= std::mem::size_of::<FillRecord>()
            {
                let fill = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const FillRecord,
                    )
                };
                let _ = fill_prod.push(FillEvent {
                    preamble: fill.preamble,
                    symbol_id: fill.symbol_id,
                    taker_user_id: fill.taker_user_id,
                    maker_user_id: fill.maker_user_id,
                    price: fill.price,
                    qty: fill.qty,
                    taker_side: fill.taker_side,
                    timestamp_ns: fill.ts_ns,
                });
            }
        }

        // Run risk engine
        shard.run_once(&mut rings, now_secs);

        // Drain responses -> CMP to Gateway
        while let Ok(resp) = resp_cons.pop() {
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &resp as *const OrderResponse
                        as *const u8,
                    std::mem::size_of::<
                        OrderResponse,
                    >(),
                )
            };
            let _ =
                gw_sender.send_record(0x20, bytes);
        }

        // CMP housekeeping
        let _ = me_sender.tick();
        let _ = gw_sender.tick();
        gw_receiver.tick();
        me_receiver.tick();
        me_sender.recv_control();
        gw_sender.recv_control();
    }
}
