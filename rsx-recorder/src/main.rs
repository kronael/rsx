mod config;

use chrono::NaiveDate;
use chrono::Utc;
use config::RecorderConfig;
use rsx_cast::ReplicationConsumer;
use rsx_cast::RawWalRecord;
use rsx_health::CounterGauge;
use rsx_health::DaemonState;
use rsx_health::HealthSnapshot;
use rsx_health::LoadGauges;
use rsx_types::install_panic_handler;
use std::env;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

struct RecorderState {
    archive_dir: PathBuf,
    stream_id: u32,
    current_date: NaiveDate,
    retain_days: i64,
    file: File,
    buf: Vec<u8>,
    record_count: u64,
}

impl RecorderState {
    fn new(
        archive_dir: &std::path::Path,
        stream_id: u32,
        retain_days: i64,
    ) -> io::Result<Self> {
        let today = Utc::now().date_naive();
        let dir = archive_dir.join(stream_id.to_string());
        fs::create_dir_all(&dir)?;

        let path = dir.join(format!(
            "{}_{}.wal", stream_id, today
        ));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        info!("recording to {}", path.display());

        prune_archive(&dir, stream_id, today, retain_days);

        Ok(Self {
            archive_dir: archive_dir.to_path_buf(),
            stream_id,
            current_date: today,
            retain_days,
            file,
            buf: Vec::with_capacity(64 * 1024),
            record_count: 0,
        })
    }

    fn write_record(
        &mut self,
        record: &RawWalRecord,
    ) -> io::Result<()> {
        let today = Utc::now().date_naive();
        if today != self.current_date {
            self.rotate(today)?;
        }

        self.buf.extend_from_slice(
            &record.header.to_bytes(),
        );
        self.buf.extend_from_slice(&record.payload);
        self.record_count += 1;

        if self.record_count.is_multiple_of(1000) {
            self.flush()?;
        }

        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        self.file.write_all(&self.buf)?;
        self.file.sync_all()?;
        self.buf.clear();
        Ok(())
    }

    fn rotate(
        &mut self,
        new_date: NaiveDate,
    ) -> io::Result<()> {
        self.flush()?;
        let dir = self.archive_dir
            .join(self.stream_id.to_string());
        let path = dir.join(format!(
            "{}_{}.wal", self.stream_id, new_date
        ));
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        self.current_date = new_date;
        info!("rotated archive to {}", path.display());
        prune_archive(&dir, self.stream_id, new_date, self.retain_days);
        Ok(())
    }
}

/// Parse the date from a `{stream_id}_{YYYY-MM-DD}.wal` segment
/// name. Returns `None` for any file that doesn't match the
/// exact archive naming pattern for this stream — the prune only
/// ever touches files it can positively identify.
fn segment_date(name: &str, stream_id: u32) -> Option<NaiveDate> {
    let prefix = format!("{}_", stream_id);
    let date_str = name
        .strip_prefix(&prefix)?
        .strip_suffix(".wal")?;
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
}

/// Delete archive segments in `dir` whose date is older than
/// `today - retain_days`. Best-effort: a bad read_dir or unlink
/// is logged, not fatal. Only files matching the exact
/// `{stream_id}_{date}.wal` pattern are considered, so the
/// active (today's) file is always kept — `today` is never
/// `< today - retain_days` for `retain_days >= 0`.
fn prune_archive(
    dir: &Path,
    stream_id: u32,
    today: NaiveDate,
    retain_days: i64,
) {
    let cutoff = today - chrono::Duration::days(retain_days);
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            warn!("prune: read_dir {} failed: {}", dir.display(), e);
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        let date = match segment_date(name, stream_id) {
            Some(d) => d,
            None => continue,
        };
        if date < cutoff {
            match fs::remove_file(&path) {
                Ok(()) => info!(
                    "pruned archive segment {} (date {}, cutoff {})",
                    path.display(), date, cutoff
                ),
                Err(e) => warn!(
                    "prune: remove_file {} failed: {}",
                    path.display(), e
                ),
            }
        }
    }
}

/// Health watchdog. The recorder's liveness IS its replication
/// progress: if no records land for `stall` seconds, the
/// replication consumer is BLOCKED (behind the producer's WAL
/// retention horizon and unable to catch up), so flip health to
/// faulted → `/health` returns 503 and the dashboard shows red.
/// Restore live/ready when progress resumes.
///
/// Idle caveat: `ReplicationConsumer` exposes no read-only
/// blocked-vs-idle signal (its public API is `new`/`run`/
/// `run_once` + a `tip` that only advances with records, i.e.
/// the same signal as `publishes`). So this is a pure write-stall
/// heuristic: a genuinely idle market with no new records would
/// false-degrade. For this demo the maker quotes constantly, so
/// write-stall ≈ blocked, which is acceptable.
async fn watchdog(gauges: Arc<LoadGauges>, stall: Duration) {
    let mut last_count = gauges.publishes.load(Ordering::Relaxed);
    let mut last_progress = Instant::now();
    let poll = Duration::from_secs(1).min(stall);
    loop {
        tokio::time::sleep(poll).await;
        let count = gauges.publishes.load(Ordering::Relaxed);
        if count != last_count {
            last_count = count;
            last_progress = Instant::now();
            if !gauges.live.load(Ordering::Relaxed) {
                gauges.live.store(true, Ordering::Relaxed);
                gauges.ready.store(true, Ordering::Relaxed);
                gauges.set_state(DaemonState::Running);
                info!("recorder recovered: records advancing again");
            }
        } else if last_progress.elapsed() >= stall
            && gauges.live.load(Ordering::Relaxed)
        {
            gauges.live.store(false, Ordering::Relaxed);
            gauges.ready.store(false, Ordering::Relaxed);
            gauges.set_state(DaemonState::Faulted);
            warn!(
                "recorder stalled: no records written for {}s, \
                 replication likely BLOCKED -> faulted",
                stall.as_secs()
            );
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = RecorderConfig::from_env()?;

    // Health server: RSX_RECORDER_HEALTH_ADDR=127.0.0.1:9205
    // GET /health → 200/503 liveness
    // GET /ready   → 200/503 readiness
    // GET /metrics → JSON (record_count, lag)
    let gauges: Arc<LoadGauges> = LoadGauges::new();
    gauges.live.store(true, Ordering::Relaxed);
    gauges.ready.store(true, Ordering::Relaxed);
    gauges.state_idx.store(4, Ordering::Relaxed); // "running"
    if let Ok(addr_str) = env::var("RSX_RECORDER_HEALTH_ADDR") {
        if let Ok(addr) = addr_str.parse::<SocketAddr>() {
            let g = gauges.clone();
            rsx_health::spawn_health_server(addr, move || {
                HealthSnapshot {
                    live: g.live.load(Ordering::Relaxed),
                    ready: g.ready.load(Ordering::Relaxed),
                    saturation: 0.0,
                    queues: vec![],
                    counters: vec![
                        CounterGauge {
                            name: "records_written",
                            value: g.publishes.load(Ordering::Relaxed),
                        },
                    ],
                    state: g.state_label(),
                }
            });
        } else {
            warn!("RSX_RECORDER_HEALTH_ADDR: invalid addr '{addr_str}'");
        }
    }

    tokio::spawn(watchdog(
        gauges.clone(),
        Duration::from_secs(config.stall_secs),
    ));

    let state = Arc::new(Mutex::new(RecorderState::new(
        &config.archive_dir,
        config.stream_id,
        config.retain_days,
    )?));

    let mut consumer = ReplicationConsumer::new(
        config.stream_id,
        vec![config.producer_addr],
        config.tip_file,
        None,
    )?;

    let state_clone = state.clone();
    let gauges_rec = gauges.clone();
    consumer
        .run(move |record: RawWalRecord| {
            // SAFETY: recover from mutex poison
            let mut s = state_clone
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Err(e) = s.write_record(&record) {
                tracing::error!(
                    "write archive error: {}", e
                );
            } else {
                gauges_rec.publishes.fetch_add(
                    1, Ordering::Relaxed,
                );
            }
            true
        })
        .await?;

    Ok(())
}

#[cfg(test)]
#[path = "main_test.rs"]
mod main_test;
