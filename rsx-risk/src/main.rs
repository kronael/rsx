use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::FillRecord;
use rsx_dxs::records::OrderCancelledRecord;
use rsx_dxs::records::OrderDoneRecord;
use rsx_dxs::records::OrderInsertedRecord;
use rsx_dxs::records::RECORD_FILL;
use rsx_dxs::records::RECORD_ORDER_CANCELLED;
use rsx_dxs::records::RECORD_ORDER_DONE;
use rsx_dxs::records::RECORD_ORDER_INSERTED;
use rsx_dxs::records::OrderFailedRecord;
use rsx_dxs::records::MarkPriceRecord;
use rsx_dxs::records::RECORD_MARK_PRICE;
use rsx_dxs::records::RECORD_ORDER_FAILED;
use rsx_dxs::records::RECORD_ORDER_REQUEST;
use rsx_matching::wire::OrderMessage;
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
use rsx_types::time::time;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
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

    // Receive mark prices from Mark process
    let mark_addr: SocketAddr =
        env::var("RSX_RISK_MARK_CMP_ADDR")
            .unwrap_or_else(|_| {
                "127.0.0.1:9105".into()
            })
            .parse()
            .expect("invalid RSX_RISK_MARK_CMP_ADDR");
    let mark_sender_addr: SocketAddr =
        env::var("RSX_MARK_CMP_ADDR")
            .unwrap_or_else(|_| {
                "127.0.0.1:9106".into()
            })
            .parse()
            .expect("invalid RSX_MARK_CMP_ADDR");
    let mut mark_receiver = CmpReceiver::new(
        mark_addr,
        mark_sender_addr,
        0,
    )
    .expect("failed to bind mark CMP receiver");

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
    let (mut mark_prod, mark_cons) =
        rtrb::RingBuffer::<MarkPriceUpdate>::new(256);
    let (_bbo_prod, bbo_cons) =
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
                    std::ptr::read(
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
                        price: fill.price,
                        qty: fill.qty,
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
                    shard.process_order_done(
                        &rsx_risk::types::OrderDoneEvent {
                            seq: rec.seq,
                            user_id: rec.user_id,
                            symbol_id: rec.symbol_id,
                            frozen_amount: 0,
                        },
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
                    price: rec.mark_price,
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
                    rsx_risk::RejectReason
                        ::InsufficientMargin => 1,
                    rsx_risk::RejectReason
                        ::UserInLiquidation => 2,
                    rsx_risk::RejectReason
                        ::NotInShard => 3,
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
                _pad1: [0; 5],
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
    }
}
