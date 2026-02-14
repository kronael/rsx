use chrono::NaiveDate;
use chrono::Utc;
use rsx_dxs::DxsConsumer;
use rsx_dxs::RawWalRecord;
use rsx_dxs::RecorderConfig;
use rsx_types::install_panic_handler;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::info;

struct RecorderState {
    archive_dir: PathBuf,
    stream_id: u32,
    current_date: NaiveDate,
    file: File,
    buf: Vec<u8>,
    record_count: u64,
}

impl RecorderState {
    fn new(
        archive_dir: &std::path::Path,
        stream_id: u32,
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

        Ok(Self {
            archive_dir: archive_dir.to_path_buf(),
            stream_id,
            current_date: today,
            file,
            buf: Vec::with_capacity(64 * 1024),
            record_count: 0,
        })
    }

    fn write_record(
        &mut self,
        record: &RawWalRecord,
    ) -> io::Result<()> {
        // check daily rotation
        let today = Utc::now().date_naive();
        if today != self.current_date {
            self.rotate(today)?;
        }

        self.buf.extend_from_slice(
            &record.header.to_bytes(),
        );
        self.buf.extend_from_slice(&record.payload);
        self.record_count += 1;

        // flush every 1000 records
        if self.record_count % 1000 == 0 {
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
        Ok(())
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    install_panic_handler();

    tracing_subscriber::fmt::init();

    let config = RecorderConfig::from_env()?;

    let state = Arc::new(Mutex::new(RecorderState::new(
        &config.archive_dir,
        config.stream_id,
    )?));

    let mut consumer = DxsConsumer::new(
        config.stream_id,
        config.producer_addr,
        config.tip_file,
        None,
    )?;

    let state_clone = state.clone();
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
            }
        })
        .await?;

    Ok(())
}
