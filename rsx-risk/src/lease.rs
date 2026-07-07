use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio_postgres::Client;
use tokio_postgres::Error;
use tracing::info;
use tracing::warn;

/// Postgres advisory-lock-backed shard lease.
///
/// Invariant #10 (at most one main per shard): the advisory lock on
/// `shard_id` is exclusive per Postgres cluster. In the eager
/// warm-standby protocol every candidate warms first, then calls the
/// NON-BLOCKING `try_acquire()` only once caught up (see
/// `main.rs::run_warm_catchup`). Postgres grants the lock to exactly
/// one caller, so two main processes for the same `shard_id` cannot
/// coexist. Catch-up gates *when* `try_acquire` is called; the lock
/// itself remains the sole single-main fence. `acquire()` (blocking)
/// is retained for callers that want to park rather than poll.
pub struct AdvisoryLease {
    shard_id: u32,
    lease_acquired: bool,
}

impl AdvisoryLease {
    pub fn new(shard_id: u32) -> Self {
        Self {
            shard_id,
            lease_acquired: false,
        }
    }

    pub async fn try_acquire(&mut self, client: &Client) -> Result<bool, Error> {
        let key = self.shard_id as i64;
        let row = client
            .query_one("SELECT pg_try_advisory_lock($1) AS acquired", &[&key])
            .await?;
        let acquired: bool = row.get(0);
        self.lease_acquired = acquired;
        if acquired {
            info!(shard_id = self.shard_id, "acquired advisory lock");
        }
        Ok(acquired)
    }

    pub async fn acquire(&mut self, client: &Client) -> Result<(), Error> {
        let key = self.shard_id as i64;
        client
            .execute("SELECT pg_advisory_lock($1)", &[&key])
            .await?;
        self.lease_acquired = true;
        info!(shard_id = self.shard_id, "acquired advisory lock");
        Ok(())
    }

    pub async fn release(&mut self, client: &Client) -> Result<(), Error> {
        if !self.lease_acquired {
            return Ok(());
        }
        let key = self.shard_id as i64;
        let row = client
            .query_one("SELECT pg_advisory_unlock($1) AS released", &[&key])
            .await?;
        let released: bool = row.get(0);
        if !released {
            warn!(
                shard_id = self.shard_id,
                "attempted to release lock not held"
            );
        } else {
            info!(shard_id = self.shard_id, "released advisory lock");
        }
        self.lease_acquired = false;
        Ok(())
    }

    pub async fn renew(&self, client: &Client) -> Result<bool, Error> {
        if !self.lease_acquired {
            return Ok(false);
        }
        let key = self.shard_id as i64;
        let row = client
            .query_one(
                "SELECT count(*) > 0 AS held \
                 FROM pg_locks WHERE locktype = 'advisory' \
                 AND objid::bigint = $1 \
                 AND pid = pg_backend_pid()",
                &[&key],
            )
            .await?;
        let held: bool = row.get(0);
        if !held {
            warn!(shard_id = self.shard_id, "lease lost unexpectedly");
        }
        Ok(held)
    }

    pub fn is_acquired(&self) -> bool {
        self.lease_acquired
    }

    pub fn shard_id(&self) -> u32 {
        self.shard_id
    }
}

/// Spawn the lease-renewal thread. Owns the promoted `rt` +
/// `pg_client` and renews the advisory lease on an interval,
/// clearing `lease_held` on loss/error.
#[allow(clippy::too_many_arguments)]
pub fn spawn_lease_thread(
    rt: tokio::runtime::Runtime,
    pg_client: Client,
    lease: AdvisoryLease,
    renew_interval_secs: u64,
    lease_held: Arc<AtomicBool>,
    lease_error: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        lease_thread_body(
            rt,
            pg_client,
            lease,
            renew_interval_secs,
            lease_held,
            lease_error,
            stop,
        )
    })
}

/// Body of the lease-renewal thread. Named (not an inline
/// `std::thread::spawn(move || {…})`) per CLAUDE.md so the
/// coroutine's lifetime is visible to the reader.
#[allow(clippy::too_many_arguments)]
fn lease_thread_body(
    rt: tokio::runtime::Runtime,
    pg_client: Client,
    mut lease: AdvisoryLease,
    renew_interval_secs: u64,
    lease_held: Arc<AtomicBool>,
    lease_error: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    rt.block_on(async move {
        let interval = Duration::from_secs(renew_interval_secs.max(1));
        let mut consec_errors: u32 = 0;
        loop {
            tokio::time::sleep(interval).await;
            if stop.load(Ordering::Relaxed) {
                if let Err(e) = lease.release(&pg_client).await {
                    warn!("lease release on stop failed: {e}");
                }
                return;
            }
            match lease.renew(&pg_client).await {
                Ok(true) => {
                    consec_errors = 0;
                }
                Ok(false) => {
                    warn!("lease lost (shard {})", lease.shard_id());
                    lease_held.store(false, Ordering::Release);
                    return;
                }
                Err(e) => {
                    consec_errors += 1;
                    warn!("lease renew error ({}/3): {e}", consec_errors);
                    if consec_errors >= 3 {
                        lease_error.store(true, Ordering::Release);
                        lease_held.store(false, Ordering::Release);
                        return;
                    }
                }
            }
        }
    });
}

/// Signal the lease thread to stop and join it.
pub fn stop_lease_thread(stop: &Arc<AtomicBool>, handle: std::thread::JoinHandle<()>) {
    stop.store(true, Ordering::Relaxed);
    if let Err(e) = handle.join() {
        warn!("lease thread panicked on join: {:?}", e);
    }
}
