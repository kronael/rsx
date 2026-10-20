//! The ME binary end to end over real sockets: orders and cancels from a
//! risk-side `CastSender` reach `process_order` / `process_cancel` through
//! the loop's dispatch, the events come back on both output streams, and
//! SIGTERM flushes the WAL (specs/2/17-matching.md:20-28, 38-69;
//! specs/2/21-orderbook.md §2.8 shutdown).

use rsx_book::event::OrderFailureReason;
use rsx_cast::as_bytes;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::WalReader;
use rsx_cast::WalHeader;
use rsx_messages::BboRecord;
use rsx_messages::CancelRequest;
use rsx_messages::FillRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderMessage;
use rsx_messages::CANCEL_REASON_USER_CANCEL;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_messages::RECORD_ORDER_REQUEST;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::net::UdpSocket;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

const SYMBOL_ID: u32 = 1;
const DEADLINE: Duration = Duration::from_secs(20);

type Frame = (u16, Vec<u8>);

struct Engine {
    child: Child,
    log: std::path::PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
        if std::thread::panicking() {
            let log = std::fs::read_to_string(&self.log).unwrap_or_default();
            eprintln!("--- matching engine log ---\n{log}");
        }
    }
}

fn free_udp_addr() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap()
}

fn free_tcp_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

fn start_engine(
    dir: &Path,
    me_addr: SocketAddr,
    risk_nak_addr: SocketAddr,
    risk_recv_addr: SocketAddr,
    mkt_addr: SocketAddr,
    health_addr: SocketAddr,
    core_id: Option<usize>,
) -> Engine {
    let log = dir.join("me.log");
    let log_file = std::fs::File::create(&log).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rsx-matching"));
    command
        .env_remove("RSX_ME_DATABASE_URL")
        .env_remove("RSX_ME_REPLICATION_BIND_ADDR")
        .env_remove("RSX_ME_CORE_ID")
        .env("RSX_ME_SYMBOL_ID", SYMBOL_ID.to_string())
        .env("RSX_ME_PRICE_DECIMALS", "0")
        .env("RSX_ME_QTY_DECIMALS", "0")
        .env("RSX_ME_TICK_SIZE", "1")
        .env("RSX_ME_LOT_SIZE", "1")
        .env("RSX_ME_WAL_DIR", dir.join("wal"))
        .env("RSX_ME_CAST_ADDR", me_addr.to_string())
        .env("RSX_RISK_ME_SEND_ADDR", risk_nak_addr.to_string())
        .env("RSX_RISK_ME_RECV_ADDR", risk_recv_addr.to_string())
        .env("RSX_MD_CAST_ADDR", mkt_addr.to_string())
        .env("RSX_ME_HEALTH_ADDR", health_addr.to_string())
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file));
    if let Some(core_id) = core_id {
        command.env("RSX_ME_CORE_ID", core_id.to_string());
    }
    let child = command.spawn().unwrap();
    Engine { child, log }
}

fn wait_until_ready(health_addr: SocketAddr) {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(health_addr) {
            stream.write_all(b"GET /ready HTTP/1.0\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            if response.starts_with("HTTP/1.1 200") {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("matching engine not ready within {DEADLINE:?}");
}

fn order_message(user_id: u32, order_id_lo: u64, side: u8, price: i64, qty: i64) -> OrderMessage {
    OrderMessage {
        seq: 0,
        price,
        qty,
        side,
        tif: 0,
        reduce_only: 0,
        post_only: 0,
        _pad1: [0; 4],
        user_id,
        _pad2: 0,
        timestamp_ns: 1,
        order_id_hi: 0,
        order_id_lo,
        risk_in_ns: 1,
    }
}

fn cancel_request(user_id: u32, order_id_lo: u64) -> CancelRequest {
    CancelRequest {
        seq: 0,
        ts_ns: 0,
        user_id,
        symbol_id: SYMBOL_ID,
        order_id_hi: 0,
        order_id_lo,
        _pad: [0; 24],
    }
}

/// The risk side's ME sender: re-sequences onto its own contiguous stream,
/// as rsx-risk does (`send_raw` + `advance_seq`).
struct RiskSide {
    sender: CastSender,
}

impl RiskSide {
    fn send_order(&mut self, mut message: OrderMessage) {
        message.seq = self.sender.next_seq();
        self.sender
            .send_raw(RECORD_ORDER_REQUEST, as_bytes(&message))
            .unwrap();
        self.sender.advance_seq();
    }

    fn send_cancel(&mut self, mut request: CancelRequest) {
        request.seq = self.sender.next_seq();
        self.sender
            .send_raw(RECORD_CANCEL_REQUEST, as_bytes(&request))
            .unwrap();
        self.sender.advance_seq();
    }
}

/// The last input is a repeat of an id cancelled earlier, so its OrderFailed
/// is the second one and the final record of the run.
fn second_order_failed() -> impl FnMut(&Frame) -> bool {
    let mut failures = 0;
    move |frame| {
        if frame.0 == RECORD_ORDER_FAILED {
            failures += 1;
        }
        failures == 2
    }
}

/// Read frames from `socket` until one satisfies `is_last` or the deadline.
fn read_until(socket: &UdpSocket, mut is_last: impl FnMut(&Frame) -> bool) -> Vec<Frame> {
    socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let deadline = Instant::now() + DEADLINE;
    let mut frames = Vec::new();
    let mut buf = [0u8; 2048];
    while Instant::now() < deadline {
        let Ok(len) = socket.recv(&mut buf) else {
            continue;
        };
        let header = WalHeader::from_bytes(&buf[..len]).unwrap();
        let frame = (header.record_type, buf[WalHeader::SIZE..len].to_vec());
        let last = is_last(&frame);
        frames.push(frame);
        if last {
            return frames;
        }
    }
    panic!(
        "last expected record not seen within {DEADLINE:?}; got {} frames",
        frames.len()
    );
}

fn decode_all<T: Copy>(frames: &[Frame], record_type: u16) -> Vec<T> {
    frames
        .iter()
        .filter(|(frame_type, _)| *frame_type == record_type)
        .map(|(_, payload)| decode_payload::<T>(payload).unwrap())
        .collect()
}

fn wait_for_exit(engine: &mut Engine) -> std::process::ExitStatus {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if let Some(status) = engine.child.try_wait().unwrap() {
            return status;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("matching engine did not exit within {DEADLINE:?} of SIGTERM");
}

/// Orders and cancels sent over casting/UDP -> the dispatch hands each to its
/// handler; WAL, risk and marketdata streams carry the same records on one
/// contiguous seq; SIGTERM leaves the WAL equal to what was published.
#[test]
fn orders_and_cancels_over_udp_reach_the_handlers_and_the_wal_survives_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let risk_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mkt_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let me_addr = free_udp_addr();
    let health_addr = free_tcp_addr();
    let sender_dir = tempfile::tempdir().unwrap();
    let sender = CastSender::new(me_addr, SYMBOL_ID, sender_dir.path()).unwrap();
    let nak_addr: SocketAddr = format!("127.0.0.1:{}", sender.local_addr().unwrap().port())
        .parse()
        .unwrap();
    let mut engine = start_engine(
        dir.path(),
        me_addr,
        nak_addr,
        risk_socket.local_addr().unwrap(),
        mkt_socket.local_addr().unwrap(),
        health_addr,
        None,
    );
    wait_until_ready(health_addr);
    let mut risk = RiskSide { sender };

    risk.send_order(order_message(1, 1, 1, 50_000, 10));
    risk.send_order(order_message(1, 1, 1, 50_000, 10));
    risk.send_order(order_message(2, 2, 0, 50_000, 4));
    risk.send_cancel(cancel_request(1, 1));
    risk.send_cancel(cancel_request(9, 99));
    risk.send_order(order_message(1, 1, 1, 50_000, 10));

    let seen_risk = read_until(&risk_socket, second_order_failed());
    let seen_mkt = read_until(&mkt_socket, second_order_failed());

    assert_eq!(seen_mkt, seen_risk);
    let types: Vec<u16> = seen_risk
        .iter()
        .map(|(record_type, _)| *record_type)
        .collect();
    assert_eq!(
        types[..4],
        [
            RECORD_ORDER_ACCEPTED,
            RECORD_ORDER_INSERTED,
            RECORD_BBO,
            RECORD_ORDER_FAILED
        ]
    );
    let seqs: Vec<u64> = seen_risk
        .iter()
        .map(|(_, payload)| extract_seq(payload).unwrap())
        .collect();
    let contiguous: Vec<u64> = (1..=seqs.len() as u64).collect();
    assert_eq!(seqs, contiguous);

    let failures = decode_all::<OrderFailedRecord>(&seen_risk, RECORD_ORDER_FAILED);
    assert_eq!(failures.len(), 2);
    for failure in &failures {
        assert_eq!(failure.reason, OrderFailureReason::Duplicate as u8);
    }
    let fills = decode_all::<FillRecord>(&seen_risk, RECORD_FILL);
    assert_eq!(fills.len(), 1);
    assert_eq!((fills[0].maker_user_id, fills[0].taker_user_id), (1, 2));
    assert_eq!(fills[0].qty.0, 4);
    let cancelled = decode_all::<OrderCancelledRecord>(&seen_risk, RECORD_ORDER_CANCELLED);
    assert_eq!(cancelled.len(), 1);
    assert_eq!(cancelled[0].remaining_qty.0, 6);
    assert_eq!(cancelled[0].reason, CANCEL_REASON_USER_CANCEL);
    assert!(!decode_all::<BboRecord>(&seen_risk, RECORD_BBO).is_empty());

    // SAFETY: plain signal to the child this test spawned.
    unsafe { libc::kill(engine.child.id() as i32, libc::SIGTERM) };
    assert!(wait_for_exit(&mut engine).success());

    let mut reader = WalReader::open_from_seq(SYMBOL_ID, 1, &dir.path().join("wal")).unwrap();
    let mut wal = Vec::new();
    while let Some(raw) = reader.next().unwrap() {
        wal.push((raw.header.record_type, raw.payload));
    }
    assert_eq!(wal, seen_risk);
}

/// `(thread name, Cpus_allowed_list)` of every thread of `pid`.
fn thread_affinities(pid: u32) -> Vec<(String, String)> {
    let mut threads = Vec::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
        let status = std::fs::read_to_string(entry.unwrap().path().join("status")).unwrap();
        let field = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .map(|value| value.trim().to_string())
                .unwrap_or_default()
        };
        threads.push((field("Name:"), field("Cpus_allowed_list:")));
    }
    threads
}

/// With `RSX_ME_CORE_ID` set only the loop thread is confined to that core;
/// the checkpoint writer, health server and log drainer, spawned before the
/// pin, can run anywhere the process may.
#[test]
fn sidecar_threads_are_not_confined_to_the_pinned_core() {
    let allowed = core_affinity::get_core_ids().unwrap();
    if allowed.len() < 2 {
        eprintln!("skipped: needs at least two allowed cpus");
        return;
    }
    let pinned_cpu = allowed[0].id.to_string();
    let dir = tempfile::tempdir().unwrap();
    let risk_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mkt_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let health_addr = free_tcp_addr();
    let engine = start_engine(
        dir.path(),
        free_udp_addr(),
        free_udp_addr(),
        risk_socket.local_addr().unwrap(),
        mkt_socket.local_addr().unwrap(),
        health_addr,
        Some(0),
    );
    wait_until_ready(health_addr);
    let pid = engine.child.id();
    let deadline = Instant::now() + DEADLINE;
    let threads = loop {
        let threads = thread_affinities(pid);
        let loop_thread = threads.iter().find(|(name, _)| name == "rsx-matching");
        if loop_thread.is_some_and(|(_, cpus)| *cpus == pinned_cpu) {
            break threads;
        }
        assert!(
            Instant::now() < deadline,
            "loop thread never pinned: {threads:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let confined: Vec<_> = threads
        .iter()
        .filter(|(name, cpus)| name != "rsx-matching" && *cpus == pinned_cpu)
        .collect();
    assert!(
        confined.is_empty(),
        "threads confined to the loop core {pinned_cpu}: {confined:?}"
    );
    assert!(
        threads
            .iter()
            .any(|(name, _)| name.starts_with("rsx-me-checkpoi")),
        "checkpoint writer thread missing: {threads:?}"
    );
}
