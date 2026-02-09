use chrono::NaiveDate;
use chrono::Utc;
use rsx_dxs::DxsConsumer;
use rsx_dxs::RawWalRecord;
use rsx_dxs::RecorderConfig;
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
        archive_dir: &PathBuf,
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
            .write(true)
            .append(true)
            .open(&path)?;

        info!("recording to {}", path.display());

        Ok(Self {
            archive_dir: archive_dir.clone(),
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
            .write(true)
            .append(true)
            .open(&path)?;
        self.current_date = new_date;
        info!("rotated archive to {}", path.display());
        Ok(())
    }
}

fn load_config(path: &str) -> io::Result<RecorderConfig> {
    let content = fs::read_to_string(path)?;
    let wrapper: toml::Value =
        toml::from_str(&content).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("toml parse: {}", e),
            )
        })?;
    let recorder = wrapper
        .get("recorder")
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "missing [recorder] section",
            )
        })?;
    let config: RecorderConfig =
        recorder.clone().try_into().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("config: {}", e),
            )
        })?;
    Ok(config)
}

#[tokio::main]
async fn main() -> io::Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "recorder.toml".to_string());
    let config = load_config(&config_path)?;

    let state = Arc::new(Mutex::new(RecorderState::new(
        &config.archive_dir,
        config.stream_id,
    )?));

    let mut consumer = DxsConsumer::new(
        config.stream_id,
        config.producer_addr,
        config.tip_file,
    );

    let state_clone = state.clone();
    consumer
        .run(move |record: RawWalRecord| {
            let mut s = state_clone.lock().unwrap();
            if let Err(e) = s.write_record(&record) {
                tracing::error!(
                    "write archive error: {}", e
                );
            }
        })
        .await?;

    Ok(())
}
