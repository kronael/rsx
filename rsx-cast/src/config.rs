//! `CastConfig` + `TlsConfig`: runtime knobs. Read once at process start.

use std::env;
use std::io;
use std::path::PathBuf;

fn env_var<T: std::str::FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Casting transport configuration.
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
    /// Max retries on the oldest gap before the receiver
    /// transitions to FAULTED and surfaces `CastRecv::Faulted`
    /// to its consumer. At 50 ms debounce × 8 = 400 ms total
    /// in-band recovery budget. Env: `RSX_CAST_MAX_NAK_RETRIES`.
    pub max_nak_retries: u16,
    /// Sender per-seq retransmit-dedup window. If a NAK arrives
    /// requesting a seq that was retransmitted within this
    /// window, the duplicate retransmit is skipped. Default
    /// 1 ms — bounds duplicate retransmits on the wire.
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
            max_nak_retries: 8,
            retx_dedup_window_us: 1000,
            nak_debounce_us: 50_000,
        }
    }
}

impl CastConfig {
    pub fn from_env() -> Self {
        Self {
            heartbeat_interval_ms: env_var("RSX_CAST_HEARTBEAT_INTERVAL_MS", 100),
            sender_bind_addr: env::var("RSX_CAST_SENDER_BIND_ADDR").ok(),
            max_nak_retries: env_var("RSX_CAST_MAX_NAK_RETRIES", 8),
            retx_dedup_window_us: env_var("RSX_CAST_RETX_DEDUP_WINDOW_US", 1000),
            nak_debounce_us: env_var("RSX_CAST_NAK_DEBOUNCE_US", 50_000),
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

/// Combined TLS configuration for replication (TCP catch-up).
///
/// Replication is TLS-mandatory (rustls + aws-lc-rs). `.server`
/// holds the cert+key a `ReplicationService` presents; `.client`
/// holds the CA a `ReplicationConsumer` trusts. `from_env`
/// populates BOTH from one `certs/` dir so a co-located
/// server+consumer self-trusts (the self-signed cert IS the CA).
///
/// The casting/UDP path stays plaintext by design (trusted LAN,
/// spec 4-cast §10.4); TLS protects only the cold TCP replay/
/// federation hop.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub server: Option<TlsServer>,
    pub client: Option<TlsClient>,
}

impl TlsConfig {
    /// Load replication TLS paths from the environment.
    ///
    /// Env vars (each defaults under a repo-root `certs/` dir):
    /// - `RSX_REPL_CERT_PATH` → `./certs/cert.pem` (server chain)
    /// - `RSX_REPL_KEY_PATH`  → `./certs/key.pem`  (server key)
    /// - `RSX_REPL_CA_PATH`   → `./certs/ca.pem`   (client trust)
    ///
    /// Errors if any file is missing — replication is
    /// TLS-mandatory, so there is no plaintext fallback.
    pub fn from_env() -> io::Result<Self> {
        let cert_path = env_path("RSX_REPL_CERT_PATH", "./certs/cert.pem");
        let key_path = env_path("RSX_REPL_KEY_PATH", "./certs/key.pem");
        let ca_path = env_path("RSX_REPL_CA_PATH", "./certs/ca.pem");

        for path in [&cert_path, &key_path, &ca_path] {
            if !path.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "replication requires TLS but {} is \
                         missing — run \
                         scripts/gen-snakeoil-certs.sh (or point \
                         RSX_REPL_CERT_PATH/KEY_PATH/CA_PATH at \
                         real certs)",
                        path.display(),
                    ),
                ));
            }
        }

        Ok(Self {
            server: Some(TlsServer {
                cert_path: cert_path.clone(),
                key_path,
            }),
            client: Some(TlsClient { cert_path: ca_path }),
        })
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    PathBuf::from(env::var(key).unwrap_or_else(|_| default.to_string()))
}
