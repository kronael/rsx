use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct DxsConfig {
    #[serde(default = "default_wal_dir")]
    pub wal_dir: PathBuf,
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
    #[serde(default = "default_retention_ns")]
    pub retention_ns: u64,
    #[serde(default = "default_flush_interval_ms")]
    pub flush_interval_ms: u64,
    #[serde(default = "default_flush_size_threshold")]
    pub flush_size_threshold: u64,
}

fn default_wal_dir() -> PathBuf {
    PathBuf::from("./wal")
}

fn default_max_file_size() -> u64 {
    64 * 1024 * 1024
}

fn default_retention_ns() -> u64 {
    10 * 60 * 1_000_000_000
}

fn default_flush_interval_ms() -> u64 {
    10
}

fn default_flush_size_threshold() -> u64 {
    1000
}

impl Default for DxsConfig {
    fn default() -> Self {
        Self {
            wal_dir: default_wal_dir(),
            max_file_size: default_max_file_size(),
            retention_ns: default_retention_ns(),
            flush_interval_ms: default_flush_interval_ms(),
            flush_size_threshold: default_flush_size_threshold(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RecorderConfig {
    pub stream_id: u32,
    pub producer_addr: String,
    pub archive_dir: PathBuf,
    pub tip_file: PathBuf,
}
