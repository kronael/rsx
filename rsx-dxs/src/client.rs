use crate::proto::dxs_replay_client::DxsReplayClient;
use crate::proto::ReplayRequest;
use crate::wal::RawWalRecord;
use crate::header::WalHeader;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tokio_stream::StreamExt;
use tracing::info;
use tracing::warn;

/// DxsConsumer: subscribes to a producer's DxsReplay stream
pub struct DxsConsumer {
    pub stream_id: u32,
    pub producer_addr: String,
    pub tip: u64,
    pub tip_file: PathBuf,
    last_tip_persist: Instant,
    tip_persist_interval: Duration,
}

impl DxsConsumer {
    pub fn new(
        stream_id: u32,
        producer_addr: String,
        tip_file: PathBuf,
    ) -> Self {
        let tip = load_tip(&tip_file).unwrap_or(0);
        info!(
            "dxs consumer stream_id={} tip={} addr={}",
            stream_id, tip, producer_addr
        );
        Self {
            stream_id,
            producer_addr,
            tip,
            tip_file,
            last_tip_persist: Instant::now(),
            tip_persist_interval: Duration::from_millis(10),
        }
    }

    /// Run the consumer loop with reconnect.
    /// Calls `callback` for each record received.
    pub async fn run<F>(
        &mut self,
        mut callback: F,
    ) -> io::Result<()>
    where
        F: FnMut(RawWalRecord),
    {
        let backoff_schedule = [1, 2, 4, 8, 30];
        let mut backoff_idx = 0;

        loop {
            match self.connect_and_stream(&mut callback).await {
                Ok(()) => {
                    info!("stream ended cleanly, reconnecting");
                    backoff_idx = 0;
                }
                Err(e) => {
                    let secs =
                        backoff_schedule[backoff_idx.min(
                            backoff_schedule.len() - 1,
                        )];
                    warn!(
                        "stream error: {}, retrying in {}s",
                        e, secs
                    );
                    tokio::time::sleep(Duration::from_secs(secs))
                        .await;
                    if backoff_idx < backoff_schedule.len() - 1 {
                        backoff_idx += 1;
                    }
                }
            }
        }
    }

    async fn connect_and_stream<F>(
        &mut self,
        callback: &mut F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(RawWalRecord),
    {
        let endpoint = format!(
            "http://{}",
            self.producer_addr
        );
        let mut client =
            DxsReplayClient::connect(endpoint).await?;

        let request = ReplayRequest {
            stream_id: self.stream_id,
            from_seq: self.tip + 1,
        };

        let response = client.stream(request).await?;
        let mut stream = response.into_inner();

        while let Some(msg) = stream.next().await {
            let msg = msg?;
            let bytes = &msg.record;

            if bytes.len() < WalHeader::SIZE {
                warn!("received undersized record");
                continue;
            }

            let header =
                WalHeader::from_bytes(bytes).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "bad header",
                    )
                })?;

            let payload =
                bytes[WalHeader::SIZE..].to_vec();

            let record = RawWalRecord { header, payload };
            callback(record);

            self.tip += 1;

            // persist tip every 10ms
            if self.last_tip_persist.elapsed()
                >= self.tip_persist_interval
            {
                persist_tip(&self.tip_file, self.tip)?;
                self.last_tip_persist = Instant::now();
            }
        }

        // persist final tip
        persist_tip(&self.tip_file, self.tip)?;
        Ok(())
    }
}

fn load_tip(path: &Path) -> io::Result<u64> {
    let data = fs::read(path)?;
    if data.len() < 8 {
        return Ok(0);
    }
    let bytes: [u8; 8] =
        data[..8].try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "bad tip file",
            )
        })?;
    Ok(u64::from_le_bytes(bytes))
}

fn persist_tip(path: &Path, tip: u64) -> io::Result<()> {
    // atomic write: write to temp, rename
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &tip.to_le_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}
