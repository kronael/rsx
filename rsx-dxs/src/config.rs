use serde::Deserialize;
use std::env;
use std::io;
use std::path::PathBuf;

fn env_var<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[derive(Debug, Deserialize, Clone)]
pub struct DxsConfig {
    #[serde(default = "default_wal_dir")]
    pub wal_dir: PathBuf,
    #[serde(default = "default_archive_dir")]
    pub archive_dir: Option<PathBuf>,
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

fn default_archive_dir() -> Option<PathBuf> {
    None
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
            archive_dir: default_archive_dir(),
            max_file_size: default_max_file_size(),
            retention_ns: default_retention_ns(),
            flush_interval_ms: default_flush_interval_ms(),
            flush_size_threshold: default_flush_size_threshold(),
        }
    }
}

impl DxsConfig {
    pub fn from_env() -> io::Result<Self> {
        Ok(Self {
            wal_dir: env::var("RSX_WAL_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| default_wal_dir()),
            archive_dir: env::var("RSX_WAL_ARCHIVE_DIR")
                .ok()
                .map(PathBuf::from),
            max_file_size: env_var(
                "RSX_WAL_MAX_FILE_SIZE",
                default_max_file_size(),
            ),
            retention_ns: env_var(
                "RSX_WAL_RETENTION_NS",
                default_retention_ns(),
            ),
            flush_interval_ms: env_var(
                "RSX_WAL_FLUSH_INTERVAL_MS",
                default_flush_interval_ms(),
            ),
            flush_size_threshold: env_var(
                "RSX_WAL_FLUSH_SIZE_THRESHOLD",
                default_flush_size_threshold(),
            ),
        })
    }
}

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

#[derive(Debug, Clone)]
pub struct CmpConfig {
    pub reorder_buf_limit: usize,
    pub heartbeat_interval_ms: u64,
    pub status_interval_ms: u64,
    pub default_window: u64,
}

impl Default for CmpConfig {
    fn default() -> Self {
        Self {
            reorder_buf_limit: 512,
            heartbeat_interval_ms: 10,
            status_interval_ms: 10,
            default_window: 64 * 1024,
        }
    }
}

impl CmpConfig {
    pub fn from_env() -> Self {
        Self {
            reorder_buf_limit: env_var(
                "RSX_CMP_REORDER_BUF_LIMIT", 512),
            heartbeat_interval_ms: env_var(
                "RSX_CMP_HEARTBEAT_INTERVAL_MS", 10),
            status_interval_ms: env_var(
                "RSX_CMP_STATUS_INTERVAL_MS", 10),
            default_window: env_var(
                "RSX_CMP_DEFAULT_WINDOW", 64 * 1024),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: Option<PathBuf>,
    pub key_path: Option<PathBuf>,
}

impl TlsConfig {
    pub fn from_env() -> Self {
        let enabled = env::var("RSX_REPL_TLS")
            .map(|v| v == "true")
            .unwrap_or(false);
        let cert_path = env::var("RSX_REPL_CERT_PATH")
            .ok()
            .map(PathBuf::from);
        let key_path = env::var("RSX_REPL_KEY_PATH")
            .ok()
            .map(PathBuf::from);
        Self {
            enabled,
            cert_path,
            key_path,
        }
    }

    pub fn validate_server(&self) -> io::Result<()> {
        if self.enabled {
            if self.cert_path.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "RSX_REPL_CERT_PATH required when TLS enabled",
                ));
            }
            if self.key_path.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "RSX_REPL_KEY_PATH required when TLS enabled",
                ));
            }
        }
        Ok(())
    }

    pub fn validate_client(&self) -> io::Result<()> {
        if self.enabled
            && self.cert_path.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "RSX_REPL_CERT_PATH required when TLS enabled",
                ));
            }
        Ok(())
    }
}
