use serde::Deserialize;
use std::env;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct RecorderConfig {
    pub stream_id: u32,
    pub producer_addr: String,
    pub archive_dir: PathBuf,
    pub tip_file: PathBuf,
}

impl RecorderConfig {
    pub fn from_env() -> io::Result<Self> {
        let stream_id = get_env_u32("RSX_RECORDER_STREAM_ID")?;
        let producer_addr =
            get_env_string("RSX_RECORDER_PRODUCER_ADDR")?;
        let archive_dir =
            get_env_path("RSX_RECORDER_ARCHIVE_DIR")?;
        let tip_file =
            get_env_path("RSX_RECORDER_TIP_FILE")?;

        Ok(Self {
            stream_id,
            producer_addr,
            archive_dir,
            tip_file,
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
