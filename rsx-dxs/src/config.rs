use std::env;
use std::io;
use std::path::PathBuf;

fn env_var<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[derive(Debug, Clone)]
pub struct CmpConfig {
    pub reorder_buf_limit: usize,
    pub heartbeat_interval_ms: u64,
    pub status_interval_ms: u64,
    pub default_window: u64,
    /// If set, CmpSender binds to this address
    /// instead of a random ephemeral port. Allows
    /// receivers to send NAKs to a known port.
    pub sender_bind_addr: Option<String>,
}

impl Default for CmpConfig {
    fn default() -> Self {
        Self {
            reorder_buf_limit: 512,
            heartbeat_interval_ms: 10,
            status_interval_ms: 10,
            default_window: 64 * 1024,
            sender_bind_addr: None,
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
            sender_bind_addr: env::var(
                "RSX_CMP_SENDER_BIND_ADDR").ok(),
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
