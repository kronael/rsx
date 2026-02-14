use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::ConfigAppliedRecord;
use rsx_dxs::records::BboRecord;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::MarkPriceRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::OrderFailedRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_BBO;
use rsx_dxs::records::RECORD_CONFIG_APPLIED;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_MARK_PRICE;
use rsx_dxs::records::RECORD_ORDER_CANCELLED;
use rsx_dxs::records::RECORD_ORDER_DONE;
use rsx_dxs::records::RECORD_ORDER_FAILED;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
use rsx_dxs::records::RECORD_ORDER_REQUEST;
use rsx_matching::wire::OrderMessage;
use rsx_risk::config::load_shard_config;
use rsx_risk::lease::AdvisoryLease;
use rsx_risk::persist::run_persist_worker;
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
use rsx_types::FailureReason;
use rsx_types::time::time;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tracing::error;
use tracing::info;
use tracing::warn;

fn log_effective_risk_config(
    config: &rsx_risk::ShardConfig,
) {
    info!(
        "risk effective config: shard_id={} shard_count={} max_symbols={} replica={} lease_poll_ms={} lease_renew_ms={} liquidation_base_delay_ns={} liquidation_base_slip_bps={} liquidation_max_rounds={}",
        config.shard_id,
        config.shard_count,
        config.max_symbols,
        config.replication_config.is_replica,
        config.replication_config.lease_poll_interval_ms,
        config.replication_config.lease_renew_interval_ms,
        config.liquidation_config.base_delay_ns,
        config.liquidation_config.base_slip_bps,
        config.liquidation_config.max_rounds,
    );
    for sid in 0..config.max_symbols {
        let p = &config.symbol_params[sid];
        info!(
            "risk symbol_config sid={} im_rate={} mm_rate={} max_leverage={} taker_fee_bps={} maker_fee_bps={}",
            sid,
            p.initial_margin_rate,
            p.maintenance_margin_rate,
            p.max_leverage,
            config.taker_fee_bps[sid],
            config.maker_fee_bps[sid],
        );
    }

    info!(
        "risk shard_routing rule='user_id % shard_count == shard_id' shard_id={} shard_count={}",
        config.shard_id,
        config.shard_count,
    );
    for user_id in 0u32..8 {
        let owner = user_id % config.shard_count;
        let serves = owner == config.shard_id;
        info!(
            "risk shard_routing_example user_id={} owner_shard={} served_here={}",
            user_id, owner, serves
        );
    }
}

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    // SAFETY: fail-fast at startup
    let config = load_shard_config()
        .expect("failed to load shard config");
    let shard_id = config.shard_id;
    let shard_count = config.shard_count;
    let max_symbols = config.max_symbols;
    let is_replica = config.replication_config.is_replica;

    info!(
        "risk shard {} starting ({} shards, {} symbols, replica={})",
        shard_id, shard_count, max_symbols, is_replica,
    );
    log_effective_risk_config(&config);

    loop {
        let result = if is_replica {
            run_replica(shard_id, max_symbols)
        } else {
            run_main(shard_id, max_symbols)
        };
        match result {
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

fn run_main(
    shard_id: u32,
    max_symbols: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_shard_config()?;
    let shard_count = config.shard_count;
    let lease_renew_interval_ms = config.replication_config.lease_renew_interval_ms;
    let mut shard = RiskShard::new(config);

    let db_url = env::var("DATABASE_URL").ok();
    let rt = if db_url.is_some() {
        Some(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        )
    } else {
        None
    };

    let mut lease = AdvisoryLease::new(shard_id);
    let pg_client = if let Some(ref url) = db_url {
        // SAFETY: rt is Some when db_url is Some
        let rt = rt.as_ref().unwrap();
        let client = rt.block_on(async {
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
            lease.acquire(&client).await?;
            let state = load_from_postgres(
                &client,
                shard_id,
                shard_count,
                max_symbols,
            )
            .await?;
            shard.load_state(state);
            info!("cold start loaded from postgres");
            Ok::<_, Box<dyn std::error::Error>>(
                client,
            )
        })?;
        Some(client)
    } else {
        None
    };

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

    let (persist_prod, persist_cons) =
        rtrb::RingBuffer::<PersistEvent>::new(8192);
    shard.set_persist_producer(persist_prod);

    // Tip sync channel for replica (if replica is running)
    let replica_addr: Option<SocketAddr> =
        env::var("RSX_RISK_REPLICA_ADDR")
            .ok()
            .and_then(|s| s.parse().ok());
    let mut tip_sender = replica_addr.map(|addr| {
        CmpSender::new(
            addr,
            0,
            Path::new(&wal_dir),
        )
        // SAFETY: fail-fast at startup
        .expect("failed to create replica tip sender")
    });

    if let Some(ref url) = db_url {
        let url = url.clone();
        let sid = shard_id;
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder
                ::new_current_thread()
                .enable_all()
                .build()
                // SAFETY: fail-fast at startup
                .expect("tokio rt");
            rt.block_on(async move {
                let (client, connection) =
                    tokio_postgres::connect(
                        &url,
                        tokio_postgres::NoTls,
                    )
                    .await
                    // SAFETY: fail-fast at startup
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

    let risk_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_CMP_ADDR");
    let gw_addr: SocketAddr =
        env::var("RSX_GW_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9102".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_GW_CMP_ADDR");
    let me_addr: SocketAddr =
        env::var("RSX_ME_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_ME_CMP_ADDR");

    let mut gw_receiver = CmpReceiver::new(
        risk_addr, gw_addr, 0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind risk CMP receiver");

    // Receive fills from ME
    let mut me_receiver = CmpReceiver::new(
        // SAFETY: literal addr is always valid
        "127.0.0.1:0".parse().expect("valid addr"),
        me_addr, 0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind ME fill receiver");

    // Receive mark prices from Mark process
    let mark_addr: SocketAddr =
        env::var("RSX_RISK_MARK_CMP_ADDR")
            .unwrap_or_else(|_| {
                "127.0.0.1:9105".into()
            })
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_MARK_CMP_ADDR");
    let mark_sender_addr: SocketAddr =
        env::var("RSX_MARK_CMP_ADDR")
            .unwrap_or_else(|_| {
                "127.0.0.1:9106".into()
            })
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_MARK_CMP_ADDR");
    let mut mark_receiver = CmpReceiver::new(
        mark_addr,
        mark_sender_addr,
        0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind mark CMP receiver");

    // Send validated orders to ME
    let mut me_sender = CmpSender::new(
        me_addr,
        0,
        Path::new(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create ME CMP sender");

    // Send responses to Gateway
    let mut gw_sender = CmpSender::new(
        gw_addr,
        0,
        Path::new(&wal_dir),
    )
    // SAFETY: fail-fast at startup
    .expect("failed to create GW CMP sender");

    // SPSC rings for run_once (internal)
    let (mut fill_prod, fill_cons) =
        rtrb::RingBuffer::<FillEvent>::new(4096);
    let (mut order_prod, order_cons) =
        rtrb::RingBuffer::<OrderRequest>::new(2048);
    let (mut mark_prod, mark_cons) =
        rtrb::RingBuffer::<MarkPriceUpdate>::new(256);
    let (mut bbo_prod, bbo_cons) =
        rtrb::RingBuffer::<BboUpdate>::new(256);
    let (resp_prod, mut resp_cons) =
        rtrb::RingBuffer::<OrderResponse>::new(2048);
    let (accepted_prod, mut accepted_cons) =
        rtrb::RingBuffer::<OrderRequest>::new(2048);

    let mut rings = ShardRings {
        fill_consumers: vec![fill_cons],
        order_consumer: order_cons,
        mark_consumer: mark_cons,
        bbo_consumers: vec![bbo_cons],
        response_producer: resp_prod,
        accepted_producer: accepted_prod,
    };

    info!("risk shard {} running", shard_id);

    let mut last_lease_renew_secs = time();
    let lease_renew_interval_secs = (lease_renew_interval_ms / 1000).max(1);

    loop {
        let now_secs = time();

        // Pump CMP -> SPSC rings
        // Orders from Gateway
        while let Some((hdr, payload)) =
            gw_receiver.try_recv()
        {
            if hdr.record_type == RECORD_ORDER_REQUEST
                && payload.len()
                    >= std::mem::size_of::<OrderRequest>()
            {
                let order = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const OrderRequest,
                    )
                };
                let _ = order_prod.push(order);
            }
        }

        // Events from ME
        while let Some((preamble, payload)) =
            me_receiver.try_recv()
        {
            match preamble.record_type {
                RECORD_BBO
                    if payload.len()
                        >= std::mem::size_of::<
                            BboRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const BboRecord,
                        )
                    };
                    let _ = bbo_prod.push(BboUpdate {
                        seq: rec.seq,
                        symbol_id: rec.symbol_id,
                        bid_px: rec.bid_px.0,
                        bid_qty: rec.bid_qty.0,
                        ask_px: rec.ask_px.0,
                        ask_qty: rec.ask_qty.0,
                    });
                }
                RECORD_FILL
                    if payload.len()
                        >= std::mem::size_of::<
                            FillRecord,
                        >() =>
                {
                    let fill = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const FillRecord,
                        )
                    };
                    let _ = fill_prod.push(FillEvent {
                        seq: fill.seq,
                        symbol_id: fill.symbol_id,
                        taker_user_id: fill
                            .taker_user_id,
                        maker_user_id: fill
                            .maker_user_id,
                        price: fill.price.0,
                        qty: fill.qty.0,
                        taker_side: fill.taker_side,
                        timestamp_ns: fill.ts_ns,
                    });
                    // Forward fill to GW
                    let _ = gw_sender.send_raw(
                        RECORD_FILL,
                        &payload,
                    );
                }
                RECORD_ORDER_DONE
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderDoneRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    OrderDoneRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    let _ = gw_sender.send_raw(
                        RECORD_ORDER_DONE,
                        &payload,
                    );
                }
                RECORD_ORDER_CANCELLED
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderCancelledRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    OrderCancelledRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    let _ = gw_sender.send_raw(
                        RECORD_ORDER_CANCELLED,
                        &payload,
                    );
                }
                RECORD_ORDER_INSERTED
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderInsertedRecord,
                        >() =>
                {
                    let _ = gw_sender.send_raw(
                        RECORD_ORDER_INSERTED,
                        &payload,
                    );
                }
                RECORD_ORDER_FAILED
                    if payload.len()
                        >= std::mem::size_of::<
                            OrderFailedRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    OrderFailedRecord,
                        )
                    };
                    shard.release_frozen_for_order(
                        rec.user_id,
                        rec.order_id_hi,
                        rec.order_id_lo,
                    );
                    let _ = gw_sender.send_raw(
                        RECORD_ORDER_FAILED,
                        &payload,
                    );
                }
                RECORD_CONFIG_APPLIED
                    if payload.len()
                        >= std::mem::size_of::<
                            ConfigAppliedRecord,
                        >() =>
                {
                    let rec = unsafe {
                        std::ptr::read_unaligned(
                            payload.as_ptr()
                                as *const
                                    ConfigAppliedRecord,
                        )
                    };
                    shard.process_config_applied(
                        rec.symbol_id,
                        rec.config_version,
                    );
                    info!(
                        "config_applied: symbol={} v={}",
                        rec.symbol_id,
                        rec.config_version,
                    );
                    let _ = gw_sender.send_raw(
                        RECORD_CONFIG_APPLIED,
                        &payload,
                    );
                }
                _ => {}
            }
        }

        // Mark prices from Mark process
        while let Some((preamble, payload)) =
            mark_receiver.try_recv()
        {
            if preamble.record_type == RECORD_MARK_PRICE
                && payload.len()
                    >= std::mem::size_of::<
                        MarkPriceRecord,
                    >()
            {
                let rec = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const MarkPriceRecord,
                    )
                };
                let _ = mark_prod.push(MarkPriceUpdate {
                    seq: rec.seq,
                    symbol_id: rec.symbol_id,
                    price: rec.mark_price.0,
                });
            }
        }

        // Run risk engine
        shard.run_once(&mut rings, now_secs);

        // Drain responses: send ORDER_FAILED to GW
        while let Ok(resp) = resp_cons.pop() {
            if let OrderResponse::Rejected {
                user_id,
                reason,
                order_id_hi,
                order_id_lo,
            } = resp
            {
                let reason_u8 = match reason {
                    rsx_risk::RejectReason::InsufficientMargin => {
                        FailureReason::InsufficientMargin
                            as u8
                    }
                    rsx_risk::RejectReason::UserInLiquidation => {
                        FailureReason::UserInLiquidation as u8
                    }
                    rsx_risk::RejectReason::NotInShard => {
                        FailureReason::WrongShard as u8
                    }
                };
                let rec = OrderFailedRecord {
                    seq: 0,
                    ts_ns: now_secs * 1_000_000_000,
                    user_id,
                    _pad0: 0,
                    order_id_hi,
                    order_id_lo,
                    reason: reason_u8,
                    _pad: [0; 23],
                };
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &rec as *const OrderFailedRecord
                            as *const u8,
                        std::mem::size_of::<
                            OrderFailedRecord,
                        >(),
                    )
                };
                let _ = gw_sender.send_raw(
                    RECORD_ORDER_FAILED,
                    bytes,
                );
            }
        }

        // Drain accepted orders -> CMP to ME
        while let Ok(order) = accepted_cons.pop() {
            let msg = OrderMessage {
                seq: order.seq,
                price: order.price,
                qty: order.qty,
                side: order.side,
                tif: order.tif,
                reduce_only: if order.reduce_only {
                    1
                } else {
                    0
                },
                post_only: if order.post_only {
                    1
                } else {
                    0
                },
                _pad1: [0; 4],
                user_id: order.user_id,
                _pad2: 0,
                timestamp_ns: order.timestamp_ns,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            };
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    &msg as *const OrderMessage
                        as *const u8,
                    std::mem::size_of::<
                        OrderMessage,
                    >(),
                )
            };
            let _ = me_sender.send_raw(
                RECORD_ORDER_REQUEST,
                bytes,
            );
        }

        // CMP housekeeping
        let _ = me_sender.tick();
        let _ = gw_sender.tick();
        gw_receiver.tick();
        me_receiver.tick();
        mark_receiver.tick();
        me_sender.recv_control();
        gw_sender.recv_control();

        // Send tips to replica if configured
        if let Some(ref mut sender) = tip_sender {
            for (symbol_id, &tip) in
                shard.tips.iter().enumerate()
            {
                // Send tip update to replica
                let tip_msg = TipSyncMessage {
                    symbol_id: symbol_id as u32,
                    tip,
                    _pad: [0; 48],
                };
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &tip_msg as *const TipSyncMessage
                            as *const u8,
                        std::mem::size_of::<
                            TipSyncMessage,
                        >(),
                    )
                };
                let _ = sender.send_raw(0x20, bytes);
            }
            let _ = sender.tick();
        }

        // Lease renewal (~1s interval)
        if now_secs - last_lease_renew_secs
            >= lease_renew_interval_secs
        {
            last_lease_renew_secs = now_secs;
            if let (Some(ref client), Some(ref rt)) =
                (&pg_client, &rt)
            {
                let held = rt.block_on(async {
                    lease.renew(client).await
                })?;
                if !held {
                    error!(
                        "lease lost, exiting for restart"
                    );
                    return Err("lease lost".into());
                }
            }
        }
    }
}

#[repr(C, align(64))]
struct TipSyncMessage {
    symbol_id: u32,
    tip: u64,
    _pad: [u8; 48],
}

fn run_replica(
    shard_id: u32,
    max_symbols: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_shard_config()?;
    let shard_count = config.shard_count;
    let mut shard = RiskShard::new(config);

    // SAFETY: fail-fast at startup
    let db_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL required for replica");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut lease = AdvisoryLease::new(shard_id);
    let client = rt.block_on(async {
        let (client, connection) =
            tokio_postgres::connect(
                &db_url,
                tokio_postgres::NoTls,
            )
            .await?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("pg connection error: {e}");
            }
        });

        // Try to acquire lock (should fail, main holds it)
        match lease.try_acquire(&client).await {
            Ok(true) => {
                warn!(
                    "replica acquired lock immediately, \
                     main not running?"
                );
            }
            Ok(false) => {
                info!(
                    "replica starting, main holds lock"
                );
            }
            Err(e) => {
                return Err(Box::new(e)
                    as Box<dyn std::error::Error>);
            }
        }

        // Load baseline state from Postgres
        let state = load_from_postgres(
            &client,
            shard_id,
            shard_count,
            max_symbols,
        )
        .await?;
        shard.load_state(state);
        info!("replica loaded baseline from postgres");
        Ok::<_, Box<dyn std::error::Error>>(client)
    })?;

    // Set up CMP receivers from MEs (same as main)
    let me_addr: SocketAddr =
        env::var("RSX_ME_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9100".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_ME_CMP_ADDR");
    let mut me_receiver = CmpReceiver::new(
        // SAFETY: literal addr is always valid
        "127.0.0.1:0".parse().expect("valid addr"),
        me_addr,
        0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind replica ME receiver");

    // Replica receives tip sync from main
    let replica_addr: SocketAddr =
        env::var("RSX_RISK_REPLICA_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9111".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_REPLICA_ADDR");
    let main_addr: SocketAddr =
        env::var("RSX_RISK_CMP_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9101".into())
            .parse()
            // SAFETY: fail-fast at startup
            .expect("invalid RSX_RISK_CMP_ADDR");
    let mut tip_receiver = CmpReceiver::new(
        replica_addr, main_addr, 0,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to bind replica tip receiver");

    let lease_poll_interval_ms = env::var(
        "RSX_RISK_LEASE_POLL_MS",
    )
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(500u64);

    let mut last_poll_ms = 0u64;
    let promoted = Arc::new(AtomicBool::new(false));

    info!(
        "replica {} running, polling for promotion",
        shard_id
    );

    loop {
        let now_secs = time();
        let now_ms = now_secs * 1000;

        // Buffer fills from ME
        while let Some((preamble, payload)) =
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
                let fill_event = FillEvent {
                    seq: fill.seq,
                    symbol_id: fill.symbol_id,
                    taker_user_id: fill.taker_user_id,
                    maker_user_id: fill.maker_user_id,
                    price: fill.price.0,
                    qty: fill.qty.0,
                    taker_side: fill.taker_side,
                    timestamp_ns: fill.ts_ns,
                };
                shard.buffer_fill_for_replica(fill_event);
            }
        }

        // Receive tip sync from main
        while let Some((preamble, payload)) =
            tip_receiver.try_recv()
        {
            if preamble.record_type == 0x20
                && payload.len()
                    >= std::mem::size_of::<
                        TipSyncMessage,
                    >()
            {
                let msg = unsafe {
                    std::ptr::read_unaligned(
                        payload.as_ptr()
                            as *const TipSyncMessage,
                    )
                };
                shard.apply_tip_from_main(
                    msg.symbol_id,
                    msg.tip,
                );
            }
        }

        // Poll for advisory lock
        if now_ms - last_poll_ms
            >= lease_poll_interval_ms
        {
            last_poll_ms = now_ms;
            let acquired = rt.block_on(async {
                lease.try_acquire(&client).await
            })?;
            if acquired {
                info!(
                    "replica acquired lock, promoting"
                );
                promoted.store(true, Ordering::Release);
                break;
            }
        }

        me_receiver.tick();
        tip_receiver.tick();
    }

    // Promotion: apply buffered fills up to last tips
    info!(
        "promoting replica to main, buffered={}",
        shard.replica_buffered_count()
    );
    let fills = shard.promote_from_replica();
    info!(
        "promotion applied {} fills, restarting as main",
        fills.len()
    );

    // After promotion, restart as main
    // lease released at scope end
    drop(client);
    std::env::set_var("RSX_RISK_IS_REPLICA", "false");
    run_main(shard_id, max_symbols)
}
