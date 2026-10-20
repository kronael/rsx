//! Composition root for the matching process: config load, Postgres
//! applied-config version, book + WAL recovery, cast sockets,
//! replication and config sidecars, health server, then hand-off to the
//! loop.

use crate::cast_loop;
use crate::cast_loop::LoopState;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalWriter;
use rsx_health::CounterGauge;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_health::QueueGauge;
use rsx_matching::checkpoint::CheckpointWriter;
use rsx_matching::config::boot_config;
use rsx_matching::config::config_link;
use rsx_matching::config::emit_config_applied;
use rsx_matching::config::load_config_from_env;
use rsx_matching::config::run_config_sidecar;
use rsx_matching::config::BootConfig;
use rsx_matching::config::ConfigSidecarEnd;
use rsx_matching::config::CONFIG_POLL_INTERVAL;
use rsx_matching::recovery::recover;
use rsx_matching::recovery::Recovered;
use rsx_types::install_shutdown_handler;
use rsx_types::time_utils::time_ns;
use rsx_types::SymbolConfig;
use std::env;
use std::fmt::Display;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio_postgres::NoTls;
use tracing::error;
use tracing::info;
use tracing::warn;

/// Exit the process with `fatal: {what}: {error}` on stderr.
fn fatal(what: &str, error: impl Display) -> ! {
    eprintln!("fatal: {what}: {error}");
    std::process::exit(1)
}

/// The socket address in the first set variable of `keys`, else `default`.
/// A malformed value panics naming its variable: boot stops.
fn env_addr(keys: &[&str], default: &str) -> SocketAddr {
    let (key, raw) = keys
        .iter()
        .find_map(|key| env::var(key).ok().map(|raw| (*key, raw)))
        .unwrap_or((keys[0], default.into()));
    raw.parse()
        .unwrap_or_else(|error| panic!("invalid {key}: {error}"))
}

/// Drive a Postgres connection until it closes. Named so the spawned
/// coroutine's lifetime is visible to the reader.
async fn drive_pg_connection(
    connection: tokio_postgres::Connection<
        tokio_postgres::Socket,
        tokio_postgres::tls::NoTlsStream,
    >,
) {
    if let Err(error) = connection.await {
        error!("db connection error: {error}");
    }
}

/// Connect to Postgres and spawn the connection driver on the current
/// runtime, which must be polled for the client to make progress.
async fn connect_db(url: &str) -> io::Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .map_err(|error| io::Error::other(format!("db connect: {error}")))?;
    tokio::spawn(drive_pg_connection(connection));
    Ok(client)
}

async fn connect_and_boot_config(
    url: &str,
    env_config: SymbolConfig,
) -> io::Result<(tokio_postgres::Client, BootConfig)> {
    let client = connect_db(url).await?;
    let boot = boot_config(&client, env_config).await?;
    Ok((client, boot))
}

/// Start the config sidecar thread: it owns the runtime and Postgres client
/// and polls for scheduled configs, so the pinned loop never waits on
/// Postgres.
fn spawn_config_sidecar(
    rt: tokio::runtime::Runtime,
    client: tokio_postgres::Client,
    symbol_id: u32,
    config_version: u64,
    link: ConfigSidecarEnd,
) {
    std::thread::Builder::new()
        .name("me-config".into())
        .spawn(move || {
            rt.block_on(run_config_sidecar(
                client,
                symbol_id,
                config_version,
                CONFIG_POLL_INTERVAL,
                link,
            ))
        })
        .expect("failed to spawn config sidecar");
}

/// Process entry after the panic handler and tracing are installed.
/// Never returns: the loop exits the process on shutdown.
pub fn run() -> ! {
    // Drain hot-path latency samples out-of-band every 100 ms.
    rsx_log::start_drainer(100);

    let initial_config =
        load_config_from_env().unwrap_or_else(|error| fatal("config error", error));

    if let Ok(core_str) = env::var("RSX_ME_CORE_ID") {
        if let Ok(core_id) = core_str.parse::<usize>() {
            let setup = rsx_types::cpu::setup_hot_thread(core_id);
            info!("me {}", setup);
            if setup.isolated == Some(false) {
                warn!("me core {} not isolated — expect tail spikes", core_id);
            }
        }
    }

    let symbol_id = initial_config.symbol_id;

    let db_url = env::var("RSX_ME_DATABASE_URL").ok();

    // With a database configured, its applied config is the book config and
    // an unreachable or unreadable database stops boot: the env config may
    // be older than what the exchange already applied. The runtime that
    // drives the connection moves to the config sidecar with the client.
    let (database, boot) = match &db_url {
        Some(url) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let (client, boot) = rt
                .block_on(connect_and_boot_config(url, initial_config))
                .unwrap_or_else(|error| fatal("boot config", error));
            (Some((rt, client)), boot)
        }
        None => {
            info!("no database url, config polling disabled");
            (None, BootConfig::env(initial_config))
        }
    };
    let config_version = boot.applied_version.unwrap_or(0);
    let book_config = boot.config;

    // Rotate at 64 MiB; hot retention (4 h) is the writer's, long-term
    // durability the archive's (specs/2/48-wal.md).
    let wal_dir = env::var("RSX_ME_WAL_DIR").unwrap_or_else(|_| "./tmp/wal".to_string());
    let mut wal_writer = WalWriter::new(symbol_id, &PathBuf::from(&wal_dir), 64 * 1024 * 1024)
        .expect("failed to create wal writer");

    // A recovery failure stops boot: trading on a half-rebuilt book or
    // re-using seqs already on disk is worse than staying down.
    let Recovered {
        book,
        order_index,
        dedup,
        next_seq,
    } = recover(boot, &wal_dir, time_ns()).unwrap_or_else(|error| fatal("recovery failed", error));
    wal_writer.set_next_seq(next_seq);

    // casting/UDP: receive orders from Risk
    let me_addr = env_addr(&["RSX_ME_CAST_ADDR"], "127.0.0.1:9100");
    // NAK destination: risk's ME sender bind addr.
    let risk_nak_addr = env_addr(
        &["RSX_RISK_ME_SEND_ADDR", "RSX_RISK_CAST_ADDR"],
        "127.0.0.1:9101",
    );
    // Risk's dedicated port for ME events (fills, BBO, etc.)
    let risk_me_recv_addr = env_addr(&["RSX_RISK_ME_RECV_ADDR"], "127.0.0.1:28301");

    let cast_receiver =
        CastReceiver::new(me_addr, risk_nak_addr).expect("failed to bind cast receiver");

    let mut cast_sender = CastSender::new(risk_me_recv_addr, symbol_id, &PathBuf::from(&wal_dir))
        .expect("failed to create cast sender");

    // casting/UDP: send events to Marketdata
    let mkt_addr = env_addr(&["RSX_MD_CAST_ADDR"], "127.0.0.1:9103");
    let mut mkt_sender = CastSender::new(mkt_addr, symbol_id, &PathBuf::from(&wal_dir))
        .expect("failed to create MD cast sender");
    info!(
        "matching effective config: symbol_id={symbol_id} tick_size={} lot_size={} price_decimals={} qty_decimals={} db_enabled={} wal_dir={wal_dir} me_cast_addr={me_addr} risk_cast_addr={risk_nak_addr} md_cast_addr={mkt_addr}",
        book_config.tick_size,
        book_config.lot_size,
        book_config.price_decimals,
        book_config.qty_decimals,
        db_url.is_some(),
    );

    // DXS sidecar
    if let Ok(dxs_addr) = env::var("RSX_ME_REPLICATION_BIND_ADDR") {
        let addr: SocketAddr = dxs_addr
            .parse()
            .expect("invalid RSX_ME_REPLICATION_BIND_ADDR");
        let wal_path = PathBuf::from(&wal_dir);
        // Replication is TLS-mandatory; read certs before the
        // thread so a missing cert fails fast at startup.
        let tls = rsx_cast::TlsConfig::from_env().expect(
            "replication requires TLS \
                 (run scripts/gen-snakeoil-certs.sh)",
        );
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime for dxs");
            let service = rsx_cast::ReplicationService::new(wal_path, tls)
                .expect("failed to create dxs service");
            rt.block_on(async {
                service.serve(addr).await.expect("dxs server failed");
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

    // SIGTERM/SIGINT flip this flag; the loop drains the WAL before exit.
    let shutdown = install_shutdown_handler();
    let checkpoint_writer =
        CheckpointWriter::start(&wal_dir, symbol_id).expect("failed to start checkpoint writer");

    let config_link = database.map(|(rt, client)| {
        let (loop_end, sidecar_end) = config_link();
        spawn_config_sidecar(rt, client, symbol_id, config_version, sidecar_end);
        loop_end
    });

    info!("matching engine started");

    cast_loop::run(
        LoopState {
            book,
            wal_writer,
            dedup,
            order_index,
            cast_receiver,
            cast_sender,
            mkt_sender,
            gauges,
            checkpoint_writer,
            symbol_id,
            config_link,
        },
        shutdown,
    )
}
