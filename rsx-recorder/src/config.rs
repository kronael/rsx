use serde::Deserialize;
use std::env;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct RecorderConfig {
    pub(crate) stream_id: u32,
    pub(crate) producer_addr: String,
    pub(crate) archive_dir: PathBuf,
    pub(crate) tip_file: PathBuf,
    /// Archive segments dated older than `today - retain_days`
    /// are pruned at startup and after each daily rotation.
    pub(crate) retain_days: i64,
    /// Flip health to faulted after this many seconds without a
    /// record written. See the watchdog in main.rs.
    pub(crate) stall_secs: u64,
}

impl RecorderConfig {
    pub(crate) fn from_env() -> io::Result<Self> {
        let stream_id = get_env_u32("RSX_RECORDER_STREAM_ID")?;
        let producer_addr =
            get_env_string("RSX_RECORDER_PRODUCER_ADDR")?;
        let archive_dir =
            get_env_path("RSX_RECORDER_ARCHIVE_DIR")?;
        let tip_file =
            get_env_path("RSX_RECORDER_TIP_FILE")?;
        let retain_days =
            get_env_u64_or("RSX_RECORDER_RETAIN_DAYS", 3)? as i64;
        let stall_secs =
            get_env_u64_or("RSX_RECORDER_STALL_SECS", 30)?;

        Ok(Self {
            stream_id,
            producer_addr,
            archive_dir,
            tip_file,
            retain_days,
            stall_secs,
        })
    }
}

fn get_env_string(key: &str) -> io::Result<String> {
    env::var(key).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing env var {}", key),
        )
    })
}

fn get_env_path(key: &str) -> io::Result<PathBuf> {
    Ok(PathBuf::from(get_env_string(key)?))
}

fn get_env_u32(key: &str) -> io::Result<u32> {
    let raw = get_env_string(key)?;
    raw.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {}: {}", key, raw),
        )
    })
}

fn get_env_u64_or(key: &str, default: u64) -> io::Result<u64> {
    match env::var(key) {
        Ok(raw) => raw.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid {}: {}", key, raw),
            )
        }),
        Err(_) => Ok(default),
    }
}
