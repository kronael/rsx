use std::env;
use std::io;
use std::path::PathBuf;

fn env_var<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// CMP transport configuration.
///
/// All fields can be overridden via env vars (see
/// [`CastConfig::from_env`]); defaults are tuned for a trusted
/// LAN with <1ms RTT.
#[derive(Debug, Clone)]
pub struct CastConfig {
    /// Sender heartbeat cadence in ms (idle-stream only —
    /// data sends reset the timer). Receivers use heartbeats
    /// to detect gaps when no data is flowing.
    /// Env: `RSX_CMP_HEARTBEAT_INTERVAL_MS`.
    pub heartbeat_interval_ms: u64,
    /// If set, CastSender binds to this address instead of a
    /// random ephemeral port. Allows receivers to send NAKs
    /// to a known port. Env: `RSX_CMP_SENDER_BIND_ADDR`.
    pub sender_bind_addr: Option<String>,
    /// Receiver NAK debounce interval. The receiver re-NAKs
    /// the oldest contiguous missing run no more frequently
    /// than this. Default 100 µs, matching typical LAN RTT.
    /// Env: `RSX_CMP_NAK_RETRY_US`.
    pub nak_retry_us: u64,
    /// Max retries on the oldest gap before the receiver
    /// transitions to FAULTED and surfaces `CastRecv::Faulted`
    /// to its consumer. At 100 µs retry × 8 = 800 µs total
    /// in-band recovery budget. Env: `RSX_CMP_MAX_NAK_RETRIES`.
    pub max_nak_retries: u16,
    /// Sender per-seq retransmit-dedup window. If a NAK arrives
    /// requesting a seq that was retransmitted within this
    /// window, the duplicate retransmit is skipped. Default
    /// 1 ms — larger than `nak_retry_us` so the layers compose.
    /// Env: `RSX_CMP_RETX_DEDUP_WINDOW_US`.
    pub retx_dedup_window_us: u64,
}

impl Default for CastConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval_ms: 100,
            sender_bind_addr: None,
            nak_retry_us: 100,
            max_nak_retries: 8,
            retx_dedup_window_us: 1000,
        }
    }
}

impl CastConfig {
    pub fn from_env() -> Self {
        Self {
            heartbeat_interval_ms: env_var(
                "RSX_CMP_HEARTBEAT_INTERVAL_MS", 100),
            sender_bind_addr: env::var(
                "RSX_CMP_SENDER_BIND_ADDR").ok(),
            nak_retry_us: env_var(
                "RSX_CMP_NAK_RETRY_US", 100),
            max_nak_retries: env_var(
                "RSX_CMP_MAX_NAK_RETRIES", 8),
            retx_dedup_window_us: env_var(
                "RSX_CMP_RETX_DEDUP_WINDOW_US", 1000),
        }
    }
}

/// TLS configuration for the DXS replay TCP path.
///
/// Defaults disable TLS; the playground and tests run plain.
/// Production deployments must enable and supply both paths
/// (server) or `cert_path` for client trust roots.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Enable TLS on the DXS replay socket.
    /// Env: `RSX_REPL_TLS` (set to `"true"`).
    pub enabled: bool,
    /// Path to the server certificate chain (PEM) — server
    /// side; or trust roots (PEM) — client side.
    /// Env: `RSX_REPL_CERT_PATH`.
    pub cert_path: Option<PathBuf>,
    /// Path to the private key (PEM). Server-only.
    /// Env: `RSX_REPL_KEY_PATH`.
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
