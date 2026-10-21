//! The gateway binary as a process: which cores its threads may run on, and
//! the `/metrics` it serves while orders flow over a real WebSocket.

use jsonwebtoken::encode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use rsx_gateway::jwt::Claims;
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

/// Start the gateway with `extra_env` on top of a minimal environment and
/// wait until `/ready` answers 200.
fn start(extra_env: &[(&str, &str)]) -> Gateway {
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
    let gateway = Gateway {
        child,
        log,
        ws_addr,
        health_addr,
        _risk: risk,
        _dir: dir,
    };
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
        let claims = Claims {
            sub: format!("github:{user_id}"),
            user_id: Some(user_id),
            exp: now + 3600,
            aud: Some("rsx-gateway".to_string()),
            iss: Some("rsx-auth".to_string()),
            nbf: None,
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap();
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
fn reactor_and_cast_recv_threads_are_pinned_to_their_own_cores() {
    let cpus = allowed_cpus();
    if cpus.len() < 2 {
        eprintln!("skipped: needs at least two allowed cpus");
        return;
    }
    let reactor_cpu = cpus[0].to_string();
    let cast_cpu = cpus[1].to_string();
    let gateway = start(&[("RSX_GW_CORE_ID", "0"), ("RSX_GW_CAST_CORE_ID", "1")]);
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
fn unset_cast_core_leaves_the_thread_unpinned_and_warns() {
    let gateway = start(&[]);
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
fn metrics_count_forwarded_and_rejected_orders() {
    let gateway = start(&[("RSX_GW_RL_USER", "3")]);
    let mut client = gateway.connect(7);
    let cid = |n: u32| format!("{n:020}");
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1000,10,"{}",0]}}"#, cid(1)),
    );
    send_text(
        &mut client,
        &format!(r#"{{"N":[99,0,1000,10,"{}",0]}}"#, cid(2)),
    );
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1001,10,"{}",0]}}"#, cid(3)),
    );
    send_text(
        &mut client,
        &format!(r#"{{"N":[1,0,1002,10,"{}",0]}}"#, cid(4)),
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
fn gateway_dies_when_the_test_process_aborts() {
    const ABORT_AFTER_START: &str = "RSX_TEST_ABORT_AFTER_START";
    const TEST_NAME: &str = "gateway_dies_when_the_test_process_aborts";
    if std::env::var_os(ABORT_AFTER_START).is_some() {
        let gateway = start(&[]);
        println!("gateway-pid={}", gateway.child.id());
        std::process::abort();
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env(ABORT_AFTER_START, "1")
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
