//! The gateway binary as a process: which cores its threads may run on, and
//! the `/metrics` it serves while orders flow over a real WebSocket.

#[path = "common/postgres.rs"]
mod postgres;

use postgres::TestPostgres;
use rsx_messages::mint_jwt;
use rsx_messages::Claims;
use rsx_types::cpu::parse_cpu_list;
use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::net::UdpSocket;
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const SECRET: &str = "process-test-secret-padded-to-32-bytes";
const DEADLINE: Duration = Duration::from_secs(20);

struct Gateway {
    child: Child,
    log: std::path::PathBuf,
    ws_addr: SocketAddr,
    health_addr: SocketAddr,
    _risk: UdpSocket,
    _dir: tempfile::TempDir,
}

impl Drop for Gateway {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
        if std::thread::panicking() {
            let log = std::fs::read_to_string(&self.log).unwrap_or_default();
            eprintln!("--- gateway log ---\n{log}");
        }
    }
}

fn kill_child_when_parent_dies(command: &mut Command) {
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: the closure runs between fork and exec and calls only
    // async-signal-safe functions (prctl, getppid).
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

fn free_tcp_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn free_udp_addr() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// Spawn the gateway with `extra_env` on top of a minimal environment that
/// reads its symbols from `database_url`.
fn spawn(database_url: &str, extra_env: &[(&str, &str)]) -> Gateway {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("gateway.log");
    let log_file = std::fs::File::create(&log).unwrap();
    let risk = UdpSocket::bind("127.0.0.1:0").unwrap();
    let ws_addr = free_tcp_addr();
    let health_addr = free_tcp_addr();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rsx-gateway"));
    command
        .env_remove("RSX_GW_CORE_ID")
        .env_remove("RSX_GW_CAST_CORE_ID")
        .env_remove("RSX_RISK_REPLICATION_ADDR")
        .env("RSX_GW_JWT_SECRET", SECRET)
        .env("RSX_GW_DATABASE_URL", database_url)
        .env("RSX_GW_LISTEN", ws_addr.to_string())
        .env("RSX_GW_CAST_ADDR", free_udp_addr().to_string())
        .env("RSX_RISK_CAST_ADDR", risk.local_addr().unwrap().to_string())
        .env("RSX_GW_WAL_DIR", dir.path().join("wal"))
        .env("RSX_GW_HEALTH_ADDR", health_addr.to_string())
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file));
    for (key, value) in extra_env {
        command.env(key, value);
    }
    kill_child_when_parent_dies(&mut command);
    let child = command.spawn().unwrap();
    Gateway {
        child,
        log,
        ws_addr,
        health_addr,
        _risk: risk,
        _dir: dir,
    }
}

/// Spawn the gateway and wait until `/ready` answers 200.
fn start(database_url: &str, extra_env: &[(&str, &str)]) -> Gateway {
    let gateway = spawn(database_url, extra_env);
    gateway.wait_until_ready();
    gateway
}

impl Gateway {
    fn wait_until_ready(&self) {
        let deadline = Instant::now() + DEADLINE;
        while Instant::now() < deadline {
            if self
                .get("/ready")
                .is_some_and(|r| r.starts_with("HTTP/1.1 200"))
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("gateway not ready within {DEADLINE:?}");
    }

    /// Full HTTP response to `GET path` on the health port.
    fn get(&self, path: &str) -> Option<String> {
        let mut stream = TcpStream::connect(self.health_addr).ok()?;
        stream
            .write_all(format!("GET {path} HTTP/1.0\r\n\r\n").as_bytes())
            .ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        Some(response)
    }

    /// Poll `/metrics` until `wanted` appears in the body.
    fn wait_for_metric(&self, wanted: &str) -> String {
        let deadline = Instant::now() + DEADLINE;
        let mut last = String::new();
        while Instant::now() < deadline {
            if let Some(response) = self.get("/metrics") {
                if response.contains(wanted) {
                    return response;
                }
                last = response;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("`{wanted}` never appeared in /metrics; last response:\n{last}");
    }

    /// Open an authenticated WebSocket for `user_id`.
    fn connect(&self, user_id: u32) -> TcpStream {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = mint_jwt(&Claims::for_user(user_id, now + 3600), SECRET).unwrap();
        let mut stream = TcpStream::connect(self.ws_addr).unwrap();
        let request = format!(
            "GET /ws/private HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\nAuthorization: Bearer {token}\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut head = [0u8; 12];
        stream.read_exact(&mut head).unwrap();
        assert_eq!(&head, b"HTTP/1.1 101", "upgrade refused");
        stream
    }
}

/// Send `text` as one masked text frame (payload under 126 bytes).
fn send_text(stream: &mut TcpStream, text: &str) {
    const MASK: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
    assert!(text.len() < 126);
    let mut frame = vec![0x81, 0x80 | text.len() as u8];
    frame.extend_from_slice(&MASK);
    frame.extend(text.bytes().enumerate().map(|(i, byte)| byte ^ MASK[i % 4]));
    stream.write_all(&frame).unwrap();
}

/// `(thread name, Cpus_allowed_list)` of every thread of `pid`.
fn thread_affinities(pid: u32) -> Vec<(String, String)> {
    let mut threads = Vec::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
        let Ok(status) = std::fs::read_to_string(entry.unwrap().path().join("status")) else {
            continue; // the thread exited between listing and reading
        };
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

/// CPUs this process may run on, ascending.
fn allowed_cpus() -> Vec<usize> {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let list = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .unwrap();
    parse_cpu_list(list.trim())
}

/// With `RSX_GW_CORE_ID` and `RSX_GW_CAST_CORE_ID` set to two cores, the
/// reactor thread runs on the first and `gw-cast-recv` on the second; the
/// health server and log drainer, spawned before either pin, can run anywhere
/// the process may.
#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn reactor_and_cast_recv_threads_are_pinned_to_their_own_cores() {
    let cpus = allowed_cpus();
    if cpus.len() < 2 {
        eprintln!("skipped: needs at least two allowed cpus");
        return;
    }
    let reactor_cpu = cpus[0].to_string();
    let cast_cpu = cpus[1].to_string();
    let postgres = TestPostgres::start_migrated();
    let gateway = start(
        &postgres.url,
        &[("RSX_GW_CORE_ID", "0"), ("RSX_GW_CAST_CORE_ID", "1")],
    );
    let pid = gateway.child.id();
    let deadline = Instant::now() + DEADLINE;
    let threads = loop {
        let threads = thread_affinities(pid);
        let on = |name: &str, cpu: &String| {
            threads
                .iter()
                .any(|(thread, cpus)| thread == name && cpus == cpu)
        };
        if on("rsx-gateway", &reactor_cpu) && on("gw-cast-recv", &cast_cpu) {
            break threads;
        }
        assert!(
            Instant::now() < deadline,
            "threads never pinned (reactor on {reactor_cpu}, gw-cast-recv on {cast_cpu}): {threads:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let confined: Vec<_> = threads
        .iter()
        .filter(|(name, cpus)| {
            name != "rsx-gateway"
                && name != "gw-cast-recv"
                && (*cpus == reactor_cpu || *cpus == cast_cpu)
        })
        .collect();
    assert!(
        confined.is_empty(),
        "sidecar threads confined to a pinned core: {confined:?}"
    );
}

/// Without `RSX_GW_CAST_CORE_ID` the recv thread is not pinned and the
/// gateway says so.
#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn unset_cast_core_leaves_the_thread_unpinned_and_warns() {
    let postgres = TestPostgres::start_migrated();
    let gateway = start(&postgres.url, &[]);
    let pid = gateway.child.id();
    let deadline = Instant::now() + DEADLINE;
    let (reactor_cpus, cast_cpus) = loop {
        let threads = thread_affinities(pid);
        let cpus_of = |name: &str| {
            threads
                .iter()
                .find(|(thread, _)| thread == name)
                .map(|(_, cpus)| cpus.clone())
        };
        if let (Some(reactor), Some(cast)) = (cpus_of("rsx-gateway"), cpus_of("gw-cast-recv")) {
            break (reactor, cast);
        }
        assert!(
            Instant::now() < deadline,
            "gw-cast-recv thread never appeared: {threads:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(cast_cpus, reactor_cpus);
    let log = std::fs::read_to_string(&gateway.log).unwrap();
    assert!(
        log.contains("RSX_GW_CAST_CORE_ID not set"),
        "no warning in the log:\n{log}"
    );
}

/// An order forwarded to risk, an order refused for an unknown symbol and one
/// refused by the user rate limit each show in `/metrics`.
#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn metrics_count_forwarded_and_rejected_orders() {
    let postgres = TestPostgres::start_migrated();
    let gateway = start(&postgres.url, &[("RSX_GW_RL_USER", "3")]);
    let mut client = gateway.connect(7);
    let cid = |n: u32| format!("{n:020}");
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1000,100,"{}",0]}}"#, cid(1)),
    );
    send_text(
        &mut client,
        &format!(r#"{{"N":[99,0,1000,100,"{}",0]}}"#, cid(2)),
    );
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1050,100,"{}",0]}}"#, cid(3)),
    );
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1100,100,"{}",0]}}"#, cid(4)),
    );

    let response = gateway.wait_for_metric(r#"{"name":"rejects_total","value":2}"#);
    assert!(response.starts_with("HTTP/1.1 200"));
    for counter in [
        r#"{"name":"orders_processed","value":2}"#,
        r#"{"name":"rejects_symbol_not_found","value":1}"#,
        r#"{"name":"rejects_rate_limit","value":1}"#,
        r#"{"name":"circuit_state","value":0}"#,
    ] {
        assert!(response.contains(counter), "missing {counter}:\n{response}");
    }
}

fn is_gone(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Err(_) => true,
        Ok(stat) => stat
            .rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z')),
    }
}

/// The test binary aborts on a panic under the cranelift backend, so
/// `Gateway::drop` does not run; the gateway still dies with the binary.
#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn gateway_dies_when_the_test_process_aborts() {
    const ABORT_AFTER_START: &str = "RSX_TEST_ABORT_AFTER_START";
    const TEST_NAME: &str = "gateway_dies_when_the_test_process_aborts";
    if let Ok(database_url) = std::env::var(ABORT_AFTER_START) {
        let gateway = start(&database_url, &[]);
        println!("gateway-pid={}", gateway.child.id());
        std::process::abort();
    }
    let postgres = TestPostgres::start_migrated();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture", "--ignored"])
        .env(ABORT_AFTER_START, &postgres.url)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid: u32 = stdout
        .split("gateway-pid=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no gateway pid in the aborted run:\n{stdout}"));
    let deadline = Instant::now() + DEADLINE;
    while !is_gone(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let leaked = !is_gone(pid);
    if leaked {
        // SAFETY: `pid` is the gateway this test started.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
    assert!(!leaked, "gateway {pid} outlived the aborted test binary");
}

/// `GET path` on the order port, the REST surface next to the WebSocket.
fn get_rest(addr: SocketAddr, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(addr).ok()?;
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\n\r\n").as_bytes())
        .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

/// `/v1/symbols` reports the decimals Postgres holds and follows a new
/// version on the poll.
#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn symbols_come_from_postgres_and_follow_a_new_version() {
    let postgres = TestPostgres::start_migrated();
    let gateway = start(&postgres.url, &[("RSX_GW_SYMBOL_POLL_S", "1")]);
    let body = get_rest(gateway.ws_addr, "/v1/symbols").unwrap();
    let pengu = r#"{"id":10,"tick_size":1,"lot_size":100000,"price_decimals":6,"qty_decimals":4,"status":"trading","min_notional":10000000000}"#;
    assert!(body.contains(pengu), "PENGU is not 6/4 in:\n{body}");
    assert!(
        !body.contains(r#""id":0,"#),
        "unlisted symbol 0 in:\n{body}"
    );

    postgres.execute(
        "INSERT INTO symbol_config_schedule (symbol_id, config_version, effective_at_ms, \
         tick_size, lot_size, price_decimals, qty_decimals, status, min_notional, created_at_ms) \
         VALUES (10, 3, 0, 1, 200000, 6, 4, 'trading', 10000000000, 0)",
    );
    let deadline = Instant::now() + DEADLINE;
    loop {
        let body = get_rest(gateway.ws_addr, "/v1/symbols").unwrap();
        if body.contains(r#""id":10,"tick_size":1,"lot_size":200000"#) {
            break;
        }
        assert!(Instant::now() < deadline, "version 3 never served:\n{body}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// With Postgres unreachable the gateway exits non-zero at boot and says why.
#[test]
fn unreachable_postgres_stops_the_boot() {
    let mut gateway = spawn("host=127.0.0.1 port=1 user=postgres connect_timeout=2", &[]);
    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = gateway.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "gateway still running");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!status.success());
    let log = std::fs::read_to_string(&gateway.log).unwrap();
    assert!(
        log.contains("failed to load symbol config from postgres"),
        "no boot error in the log:\n{log}"
    );
}

/// Without `RSX_GW_DATABASE_URL` the gateway refuses to start.
#[test]
fn missing_database_url_stops_the_boot() {
    let mut gateway = spawn("", &[]);
    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = gateway.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "gateway still running");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(2));
}

/// Read one server frame as opcode and payload.
fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut preamble = [0u8; 2];
    stream.read_exact(&mut preamble).unwrap();
    let mut payload = vec![0u8; usize::from(preamble[1] & 0x7F)];
    stream.read_exact(&mut payload).unwrap();
    (preamble[0] & 0x0F, payload)
}

/// SIGTERM with one order awaiting risk: `/ready` turns false, a new order
/// gets `E[5]`, the unanswered order gets its TIMEOUT update, then the client
/// gets close 1001 and the process exits 0 within the deadline.
#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn sigterm_drains_then_closes_sessions_and_exits() {
    let postgres = TestPostgres::start_migrated();
    let mut gateway = start(&postgres.url, &[("RSX_GW_ORDER_TIMEOUT_MS", "500")]);
    let mut client = gateway.connect(7);
    let (opcode, _) = read_frame(&mut client);
    assert_eq!(opcode, 1);
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1000,100,"{:020}",0]}}"#, 1),
    );
    gateway.wait_for_metric(r#"{"name":"orders_processed","value":1}"#);

    // SAFETY: the pid is the gateway this test started.
    unsafe { libc::kill(gateway.child.id() as libc::pid_t, libc::SIGTERM) };
    gateway.wait_for_metric(r#"{"name":"draining","value":1}"#);
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1050,100,"{:020}",0]}}"#, 2),
    );

    let mut frames = Vec::new();
    loop {
        let (opcode, payload) = read_frame(&mut client);
        if opcode == 8 {
            assert_eq!(payload, 1001u16.to_be_bytes());
            break;
        }
        frames.push(String::from_utf8(payload).unwrap());
    }
    assert!(
        frames.iter().any(|f| f == r#"{"E":[5,"overloaded"]}"#),
        "no E[5] in {frames:?}"
    );
    assert!(
        frames
            .iter()
            .any(|f| f.starts_with(r#"{"U":["#) && f.contains(",3,0,0,10,")),
        "no TIMEOUT update in {frames:?}"
    );

    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        if let Some(status) = gateway.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "gateway still running");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
}
