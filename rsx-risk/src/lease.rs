use tokio_postgres::Client;
use tokio_postgres::Error;
use tracing::info;
use tracing::warn;

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

    pub async fn try_acquire(
        &mut self,
        client: &Client,
    ) -> Result<bool, Error> {
        let key = self.shard_id as i64;
        let row = client
            .query_one(
                "SELECT pg_try_advisory_lock($1) AS acquired",
                &[&key],
            )
            .await?;
        let acquired: bool = row.get(0);
        self.lease_acquired = acquired;
        if acquired {
            info!(shard_id = self.shard_id, "acquired advisory lock");
        }
        Ok(acquired)
    }

    pub async fn acquire(
        &mut self,
        client: &Client,
    ) -> Result<(), Error> {
        let key = self.shard_id as i64;
        client
            .execute("SELECT pg_advisory_lock($1)", &[&key])
            .await?;
        self.lease_acquired = true;
        info!(shard_id = self.shard_id, "acquired advisory lock");
        Ok(())
    }

    pub async fn release(
        &mut self,
        client: &Client,
    ) -> Result<(), Error> {
        if !self.lease_acquired {
            return Ok(());
        }
        let key = self.shard_id as i64;
        let row = client
            .query_one(
                "SELECT pg_advisory_unlock($1) AS released",
                &[&key],
            )
            .await?;
        let released: bool = row.get(0);
        if !released {
            warn!(
                shard_id = self.shard_id,
                "attempted to release lock not held"
            );
        } else {
            info!(
                shard_id = self.shard_id,
                "released advisory lock"
            );
        }
        self.lease_acquired = false;
        Ok(())
    }

    pub async fn renew(
        &self,
        client: &Client,
    ) -> Result<bool, Error> {
        if !self.lease_acquired {
            return Ok(false);
        }
        let key = self.shard_id as i64;
        let row = client
            .query_one(
                "SELECT count(*) > 0 AS held \
                 FROM pg_locks WHERE locktype = 'advisory' \
                 AND objid = $1 AND pid = pg_backend_pid()",
                &[&key],
            )
            .await?;
        let held: bool = row.get(0);
        if !held {
            warn!(
                shard_id = self.shard_id,
                "lease lost unexpectedly"
            );
        }
        Ok(held)
    }

    pub fn is_acquired(&self) -> bool {
        self.lease_acquired
    }
}
