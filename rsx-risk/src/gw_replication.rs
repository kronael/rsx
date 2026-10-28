//! The gateway stream's replication listener: it serves the shard's WAL
//! directory to the gateway's replay while this node is main, on its own
//! named side-thread.

use rsx_cast::ReplicationService;
use rsx_cast::TlsConfig;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use tokio::runtime::Runtime;
use tracing::warn;

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(50);
const BIND_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const BIND_WARN_EVERY: u32 = 10;

pub struct GwReplication {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl GwReplication {
    /// Spawn the `gw-replication` thread serving `wal_dir` on `addr`. A busy
    /// port is retried until `deadline`, since the previous main closes its
    /// listener moments after the lock; any other bind error, or the
    /// deadline, panics the thread and the panic hook exits the process.
    pub fn start(
        wal_dir: PathBuf,
        tls: TlsConfig,
        addr: SocketAddr,
        deadline: Instant,
    ) -> io::Result<Self> {
        let service = ReplicationService::new(wal_dir, tls)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("gw-replication".into())
            .spawn(move || serve_loop(service, addr, deadline, &thread_stop))?;
        Ok(Self { stop, thread })
    }

    /// Close the listener and every client connection, then join. The port
    /// is free again when this returns.
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        if self.thread.join().is_err() {
            warn!("gw replication thread panicked");
        }
    }
}

fn serve_loop(service: ReplicationService, addr: SocketAddr, deadline: Instant, stop: &AtomicBool) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime for gw replication");
    let mut busy_retries: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        let task = runtime.spawn(service.clone().serve(addr));
        if !wait_until_finished(&runtime, &task, stop) {
            return;
        }
        let result = runtime
            .block_on(task)
            .expect("gw replication serve task panicked");
        let Err(error) = result else {
            return;
        };
        if error.kind() != io::ErrorKind::AddrInUse {
            panic!("gw replication {addr}: {error}");
        }
        if Instant::now() >= deadline {
            panic!("gw replication {addr} still in use at the bind deadline: {error}");
        }
        if busy_retries.is_multiple_of(BIND_WARN_EVERY) {
            warn!("risk: gw replication {addr} is in use, retrying");
        }
        busy_retries += 1;
        std::thread::sleep(BIND_RETRY_INTERVAL);
    }
}

fn wait_until_finished(
    runtime: &Runtime,
    task: &tokio::task::JoinHandle<io::Result<()>>,
    stop: &AtomicBool,
) -> bool {
    while !task.is_finished() {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        runtime.block_on(async { tokio::time::sleep(STOP_POLL_INTERVAL).await });
    }
    true
}
