use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::ReplayRequest;
use crate::records::RECORD_REPLAY_REQUEST;
use crate::wal::RawWalRecord;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::info;
use tracing::warn;

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
            "dxs consumer stream_id={} tip={} \
             addr={}",
            stream_id, tip, producer_addr
        );
        Self {
            stream_id,
            producer_addr,
            tip,
            tip_file,
            last_tip_persist: Instant::now(),
            tip_persist_interval:
                Duration::from_millis(10),
        }
    }

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
            match self
                .connect_and_stream(&mut callback)
                .await
            {
                Ok(()) => {
                    info!(
                        "stream ended, reconnecting"
                    );
                    backoff_idx = 0;
                }
                Err(e) => {
                    let secs = backoff_schedule
                        [backoff_idx.min(
                            backoff_schedule.len()
                                - 1,
                        )];
                    warn!(
                        "stream error: {}, \
                         retrying in {}s",
                        e, secs
                    );
                    tokio::time::sleep(
                        Duration::from_secs(secs),
                    )
                    .await;
                    if backoff_idx
                        < backoff_schedule.len() - 1
                    {
                        backoff_idx += 1;
                    }
                }
            }
        }
    }

    async fn connect_and_stream<F>(
        &mut self,
        callback: &mut F,
    ) -> io::Result<()>
    where
        F: FnMut(RawWalRecord),
    {
        let mut stream = TcpStream::connect(
            &self.producer_addr,
        )
        .await?;

        // Send ReplayRequest as WAL record
        let req = ReplayRequest {
            stream_id: self.stream_id,
            _pad0: 0,
            from_seq: self.tip + 1,
            _pad1: [0u8; 48],
        };
        let payload = unsafe {
            std::slice::from_raw_parts(
                &req as *const ReplayRequest
                    as *const u8,
                std::mem::size_of::<
                    ReplayRequest,
                >(),
            )
        };
        let crc = compute_crc32(payload);
        let hdr = WalHeader::new(
            RECORD_REPLAY_REQUEST,
            payload.len() as u16,
            crc,
        );
        stream
            .write_all(&hdr.to_bytes())
            .await?;
        stream.write_all(payload).await?;

        // Read WAL records
        let mut hdr_buf = [0u8; WalHeader::SIZE];
        loop {
            match stream
                .read_exact(&mut hdr_buf)
                .await
            {
                Ok(_) => {}
                Err(ref e)
                    if e.kind()
                        == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }

            let header =
                WalHeader::from_bytes(&hdr_buf)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "bad header",
                        )
                    })?;

            let payload_len = header.len as usize;
            let mut payload =
                vec![0u8; payload_len];
            stream
                .read_exact(&mut payload)
                .await?;

            let record =
                RawWalRecord { header, payload };
            callback(record);

            self.tip += 1;

            if self.last_tip_persist.elapsed()
                >= self.tip_persist_interval
            {
                persist_tip(
                    &self.tip_file,
                    self.tip,
                )?;
                self.last_tip_persist =
                    Instant::now();
            }
        }

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

fn persist_tip(
    path: &Path,
    tip: u64,
) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, tip.to_le_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}
