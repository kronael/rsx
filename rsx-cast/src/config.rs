use std::env;
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
    /// Env: `RSX_CAST_HEARTBEAT_INTERVAL_MS`.
    pub heartbeat_interval_ms: u64,
    /// If set, CastSender binds to this address instead of a
    /// random ephemeral port. Allows receivers to send NAKs
    /// to a known port. Env: `RSX_CAST_SENDER_BIND_ADDR`.
    pub sender_bind_addr: Option<String>,
    /// Receiver NAK debounce interval. The receiver re-NAKs
    /// the oldest contiguous missing run no more frequently
    /// than this. Default 100 µs, matching typical LAN RTT.
    /// Env: `RSX_CAST_NAK_RETRY_US`.
    pub nak_retry_us: u64,
    /// Max retries on the oldest gap before the receiver
    /// transitions to FAULTED and surfaces `CastRecv::Faulted`
    /// to its consumer. At 100 µs retry × 8 = 800 µs total
    /// in-band recovery budget. Env: `RSX_CAST_MAX_NAK_RETRIES`.
    pub max_nak_retries: u16,
    /// Sender per-seq retransmit-dedup window. If a NAK arrives
    /// requesting a seq that was retransmitted within this
    /// window, the duplicate retransmit is skipped. Default
    /// 1 ms — larger than `nak_retry_us` so the layers compose.
    /// Env: `RSX_CAST_RETX_DEDUP_WINDOW_US`.
    pub retx_dedup_window_us: u64,
    /// Receiver per-gap NAK debounce window. Once a NAK has
    /// been sent for a given gap (`from_seq`), the receiver
    /// won't re-NAK that same gap for this many µs. Prevents
    /// NAK storms on persistent gaps. Default 50 ms.
    /// Env: `RSX_CAST_NAK_DEBOUNCE_US`.
    pub nak_debounce_us: u64,
}

impl Default for CastConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval_ms: 100,
            sender_bind_addr: None,
            nak_retry_us: 100,
            max_nak_retries: 8,
            retx_dedup_window_us: 1000,
            nak_debounce_us: 50_000,
        }
    }
}

impl CastConfig {
    pub fn from_env() -> Self {
        Self {
            heartbeat_interval_ms: env_var(
                "RSX_CAST_HEARTBEAT_INTERVAL_MS", 100),
            sender_bind_addr: env::var(
                "RSX_CAST_SENDER_BIND_ADDR").ok(),
            nak_retry_us: env_var(
                "RSX_CAST_NAK_RETRY_US", 100),
            max_nak_retries: env_var(
                "RSX_CAST_MAX_NAK_RETRIES", 8),
            retx_dedup_window_us: env_var(
                "RSX_CAST_RETX_DEDUP_WINDOW_US", 1000),
            nak_debounce_us: env_var(
                "RSX_CAST_NAK_DEBOUNCE_US", 50_000),
        }
    }
}

/// TLS for the replication server (cert chain + private key).
#[derive(Debug, Clone)]
pub struct TlsServer {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

/// TLS for the replication client (trust root / CA cert).
#[derive(Debug, Clone)]
pub struct TlsClient {
    pub cert_path: PathBuf,
}

/// Combined TLS configuration — `Some` means TLS is active.
///
/// `.server` is set when `RSX_REPL_KEY_PATH` is present;
/// `.client` is set whenever `RSX_REPL_CERT_PATH` is present.
/// Pass `Option<TlsConfig>` to both `ReplicationService::new`
/// and `ReplicationConsumer::new`; each side picks its field.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub server: Option<TlsServer>,
    pub client: Option<TlsClient>,
}

impl TlsConfig {
    /// Returns `None` if `RSX_REPL_TLS != "true"`.
    pub fn from_env() -> Option<Self> {
        if env::var("RSX_REPL_TLS").as_deref() != Ok("true") {
            return None;
        }
        let cert_path = PathBuf::from(
            env::var("RSX_REPL_CERT_PATH")
                .expect(
                    "RSX_REPL_CERT_PATH required                      when RSX_REPL_TLS=true",
                ),
        );
        let server = env::var("RSX_REPL_KEY_PATH")
            .ok()
            .map(|kp| TlsServer {
                cert_path: cert_path.clone(),
                key_path: PathBuf::from(kp),
            });
        Some(Self {
            server,
            client: Some(TlsClient { cert_path }),
        })
    }
}
