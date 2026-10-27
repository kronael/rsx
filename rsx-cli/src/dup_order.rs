use crate::cluster::log_tail;
use crate::cluster::shutdown_signal;
use crate::output::Failure;
use crate::output::Outcome;
use crate::probe;
use crate::probe::Probe;
use crate::procs;
use crate::record;
use crate::runtime_utils::block_on;
use rsx_book::event::OrderFailureReason;
use rsx_cast::as_bytes;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalReader;
use rsx_matching::wal::failure_reason_code;
use rsx_messages::OrderMessage;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::time_utils::time_ns;
use serde_json::json;
use serde_json::Value;
use std::fs;
use std::fs::File;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use tokio::time::sleep;
use tokio::time::Instant;

#[cfg(test)]
#[path = "dup_order_test.rs"]
mod dup_order_test;

/// The symbol of the scratch matching engine; the live engines never see it.
pub const SCRATCH_SYMBOL_ID: u32 = 999;

const READY_TIMEOUT: Duration = Duration::from_secs(15);
const DUPLICATE_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_GRACE: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(50);
const ORDER_USER_ID: u32 = 1;
const ORDER_PRICE: i64 = 100;
const ORDER_QTY: i64 = 1;
const SELL: u8 = 1;
const ORDER_ACCEPTED: &str = "ORDER_ACCEPTED";
const ORDER_FAILED: &str = "ORDER_FAILED";

/// Where the scratch engine runs: the binary under `root`, its WAL, log and
/// the sender's directory under `dir`.
pub struct Options {
    pub root: PathBuf,
    pub release: bool,
    pub dir: PathBuf,
}

/// What the scratch engine's WAL holds for the order that was sent twice.
pub struct Duplicate {
    pub order_id: String,
    pub records: Vec<Value>,
}

/// Starts a scratch matching engine on its own ports, symbol and WAL, casts
/// the same `OrderMessage` to it twice the way risk does, and prints the WAL
/// records of that order: one ORDER_ACCEPTED and one ORDER_FAILED
/// (duplicate). The live engines are not touched.
pub fn duplicate_order(options: &Options, is_json: bool) -> Outcome {
    let duplicate = block_on(async {
        tokio::select! {
            duplicate = do_duplicate_order(options) => duplicate,
            _ = shutdown_signal() => Err(Failure::runtime(
                "interrupted: the scratch matching engine is stopped"
            )),
        }
    })?;
    if is_json {
        println!(
            "{}",
            json!({
                "sent": 2,
                "order_id": duplicate.order_id,
                "records": duplicate.records,
            })
        );
    } else {
        println!("sent order {} twice", duplicate.order_id);
        for record in &duplicate.records {
            println!("{}", describe(record));
        }
    }
    Ok(())
}

fn describe(record: &Value) -> String {
    let mut line = format!(
        "seq={} type={}",
        record["seq"],
        record["type"].as_str().unwrap_or("?")
    );
    if let Some(reason) = record.get("reason") {
        line.push_str(&format!(" reason={reason}"));
    }
    line
}

async fn do_duplicate_order(options: &Options) -> Result<Duplicate, Failure> {
    let profile = if options.release { "release" } else { "debug" };
    let binary = options.root.join(format!("target/{profile}/rsx-matching"));
    if !binary.exists() {
        return Err(Failure::runtime(format!(
            "binary not found: {} (build it first)",
            binary.display()
        )));
    }
    fs::create_dir_all(&options.dir).map_err(|error| {
        Failure::runtime(format!(
            "failed to create {}: {error}",
            options.dir.display()
        ))
    })?;
    let wal_dir = options.dir.join("wal");
    let log = options.dir.join("me.log");
    let peers = Peers::bind()?;
    let me_addr = free_udp_addr()?;
    let health_addr = free_tcp_addr()?;
    let env = scratch_env(&wal_dir, me_addr, &peers.addrs, &health_addr);
    let mut engine = Engine::spawn(&binary, &env, &log)?;
    engine.wait_ready(&health_addr, &log).await?;
    let order_id_lo = time_ns() & i64::MAX as u64;
    send_twice(me_addr, &options.dir.join("sender"), order_id_lo)?;
    engine.wait_duplicate(&health_addr, &log).await?;
    engine.stop()?;
    let order_id = order_id_hex(order_id_lo);
    let records = read_order_records(&wal_dir, &order_id)?;
    check_outcome(&records, &order_id)?;
    Ok(Duplicate { order_id, records })
}

/// The three UDP sockets the scratch engine sends to; held so its sends are
/// not refused.
struct Peers {
    _sockets: Vec<UdpSocket>,
    addrs: [SocketAddr; 3],
}

impl Peers {
    fn bind() -> Result<Self, Failure> {
        let mut sockets = Vec::new();
        let mut addrs = Vec::new();
        for _ in 0..3 {
            let socket = UdpSocket::bind("127.0.0.1:0")
                .map_err(|error| Failure::runtime(format!("failed to bind a socket: {error}")))?;
            addrs.push(socket.local_addr().map_err(|error| {
                Failure::runtime(format!("failed to read a socket address: {error}"))
            })?);
            sockets.push(socket);
        }
        let addrs = [addrs[0], addrs[1], addrs[2]];
        Ok(Self {
            _sockets: sockets,
            addrs,
        })
    }
}

fn free_udp_addr() -> Result<SocketAddr, Failure> {
    let socket = UdpSocket::bind("127.0.0.1:0")
        .map_err(|error| Failure::runtime(format!("failed to bind a socket: {error}")))?;
    socket
        .local_addr()
        .map_err(|error| Failure::runtime(format!("failed to read a socket address: {error}")))
}

fn free_tcp_addr() -> Result<String, Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| Failure::runtime(format!("failed to bind a socket: {error}")))?;
    let addr = listener
        .local_addr()
        .map_err(|error| Failure::runtime(format!("failed to read a socket address: {error}")))?;
    Ok(addr.to_string())
}

/// The complete environment of the scratch engine: nothing is inherited, so
/// no database url reaches it and its config comes from here.
pub(crate) fn scratch_env(
    wal_dir: &Path,
    me_addr: SocketAddr,
    peers: &[SocketAddr; 3],
    health_addr: &str,
) -> Vec<(&'static str, String)> {
    let [nak_addr, risk_addr, md_addr] = peers;
    vec![
        ("RSX_ME_SYMBOL_ID", SCRATCH_SYMBOL_ID.to_string()),
        ("RSX_ME_PRICE_DECIMALS", "0".to_string()),
        ("RSX_ME_QTY_DECIMALS", "0".to_string()),
        ("RSX_ME_TICK_SIZE", "1".to_string()),
        ("RSX_ME_LOT_SIZE", "1".to_string()),
        ("RSX_ME_BOOK_CAPACITY", "4096".to_string()),
        ("RSX_ME_WAL_DIR", wal_dir.display().to_string()),
        ("RSX_ME_CAST_ADDR", me_addr.to_string()),
        ("RSX_RISK_ME_SEND_ADDR", nak_addr.to_string()),
        ("RSX_RISK_ME_RECV_ADDR", risk_addr.to_string()),
        ("RSX_MD_CAST_ADDR", md_addr.to_string()),
        ("RSX_ME_HEALTH_ADDR", health_addr.to_string()),
    ]
}

/// The scratch matching engine process. It dies with this process: stopped
/// on drop, and sent SIGTERM by the kernel if this process is killed.
struct Engine {
    child: Child,
}

impl Engine {
    fn spawn(binary: &Path, env: &[(&str, String)], log: &Path) -> Result<Self, Failure> {
        let log_file = File::create(log).map_err(|error| {
            Failure::runtime(format!("failed to open {}: {error}", log.display()))
        })?;
        let log_for_stderr = log_file.try_clone().map_err(|error| {
            Failure::runtime(format!("failed to open {}: {error}", log.display()))
        })?;
        let mut command = Command::new(binary);
        command
            .env_clear()
            .envs(env.iter().map(|(key, value)| (*key, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_for_stderr));
        procs::die_with_parent_on_spawn(&mut command);
        let child = command.spawn().map_err(|error| {
            Failure::runtime(format!("failed to start {}: {error}", binary.display()))
        })?;
        Ok(Self { child })
    }

    fn has_exited(&mut self) -> Result<Option<String>, Failure> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| Failure::runtime(format!("failed to poll the engine: {error}")))?;
        Ok(status.map(|status| status.to_string()))
    }

    async fn wait_ready(&mut self, health_addr: &str, log: &Path) -> Result<(), Failure> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.has_exited()? {
                return Err(self.failure(&format!("exited during boot ({status})"), log));
            }
            if probe::is_up(health_addr, Probe::Ready).await {
                return Ok(());
            }
            sleep(POLL).await;
        }
        Err(self.failure("not ready", log))
    }

    async fn wait_duplicate(&mut self, health_addr: &str, log: &Path) -> Result<(), Failure> {
        let deadline = Instant::now() + DUPLICATE_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.has_exited()? {
                return Err(self.failure(&format!("exited ({status})"), log));
            }
            let rejected = probe::counter(health_addr, "rejects_duplicate").await;
            if rejected.is_some_and(|count| count >= 1) {
                return Ok(());
            }
            sleep(POLL).await;
        }
        Err(self.failure("did not reject the second copy", log))
    }

    fn failure(&self, what: &str, log: &Path) -> Failure {
        Failure::runtime(format!(
            "scratch matching engine {what}:\n{}",
            log_tail(log)
        ))
    }

    fn stop(&mut self) -> Result<(), Failure> {
        procs::terminate(&mut self.child, STOP_GRACE).map_err(|error| {
            Failure::runtime(format!(
                "failed to stop the scratch matching engine: {error}"
            ))
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Err(error) = procs::terminate(&mut self.child, STOP_GRACE) {
            eprintln!(
                "failed to stop the scratch matching engine (pid {}): {error}",
                self.child.id()
            );
        }
    }
}

/// The order risk would cast to a matching engine, with `seq` the next one
/// of that engine's cast stream.
pub(crate) fn order_message(seq: u64, order_id_lo: u64, now_ns: u64) -> OrderMessage {
    OrderMessage {
        seq,
        price: ORDER_PRICE,
        qty: ORDER_QTY,
        side: SELL,
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        _pad1: [0; 4],
        user_id: ORDER_USER_ID,
        _pad2: 0,
        timestamp_ns: now_ns,
        order_id_hi: 0,
        order_id_lo,
        risk_in_ns: now_ns,
    }
}

fn send_twice(me_addr: SocketAddr, sender_dir: &Path, order_id_lo: u64) -> Result<(), Failure> {
    let failed = |error: std::io::Error| {
        Failure::runtime(format!("failed to cast the order to {me_addr}: {error}"))
    };
    let mut sender = CastSender::new(me_addr, SCRATCH_SYMBOL_ID, sender_dir).map_err(failed)?;
    for _ in 0..2 {
        let message = order_message(sender.next_seq(), order_id_lo, time_ns());
        sender
            .send_raw(RECORD_ORDER_REQUEST, as_bytes(&message))
            .map_err(failed)?;
        sender.advance_seq();
    }
    Ok(())
}

pub(crate) fn order_id_hex(order_id_lo: u64) -> String {
    format!("{:016x}{:016x}", 0, order_id_lo)
}

/// The WAL records of the scratch stream whose `oid` is `order_id`, as
/// `wal dump --json` prints them.
fn read_order_records(wal_dir: &Path, order_id: &str) -> Result<Vec<Value>, Failure> {
    let failed = |error: std::io::Error| {
        Failure::runtime(format!(
            "failed to read the scratch WAL {}: {error}",
            wal_dir.display()
        ))
    };
    let mut reader = WalReader::open_from_seq(SCRATCH_SYMBOL_ID, 1, wal_dir).map_err(failed)?;
    let mut records = Vec::new();
    while let Some(raw) = reader.next().map_err(failed)? {
        let record = record::to_json(&raw);
        if record["oid"] == order_id {
            records.push(record);
        }
    }
    Ok(records)
}

/// Fails unless the records hold exactly one ORDER_ACCEPTED followed by one
/// ORDER_FAILED carrying the duplicate reason.
pub(crate) fn check_outcome(records: &[Value], order_id: &str) -> Result<(), Failure> {
    let seqs_of = |name: &str| -> Vec<u64> {
        records
            .iter()
            .filter(|record| record["type"] == name)
            .filter_map(|record| record["seq"].as_u64())
            .collect()
    };
    let accepted = seqs_of(ORDER_ACCEPTED);
    let failed = seqs_of(ORDER_FAILED);
    let duplicate = u64::from(failure_reason_code(OrderFailureReason::Duplicate));
    let is_duplicate = records
        .iter()
        .filter(|record| record["type"] == ORDER_FAILED)
        .all(|record| record["reason"].as_u64() == Some(duplicate));
    let is_ordered = matches!((accepted.first(), failed.first()), (Some(a), Some(f)) if a < f);
    if accepted.len() == 1 && failed.len() == 1 && is_duplicate && is_ordered {
        return Ok(());
    }
    let seen: Vec<String> = records.iter().map(describe).collect();
    Err(Failure::runtime(format!(
        "expected one ORDER_ACCEPTED and then one ORDER_FAILED with reason {duplicate} \
         for order {order_id}; the WAL has [{}]",
        seen.join(", ")
    )))
}
