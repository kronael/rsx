//! `ReplicationConsumer`: TCP catch-up client. See `specs/2/10-replication.md`.

use crate::config::TlsConfig;
use crate::encode_utils::as_bytes;
use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::ReplicationRequest;
use crate::records::RECORD_REPLICATION_NOT_AVAILABLE;
use crate::records::RECORD_REPLICATION_REQUEST;
use crate::tls::build_connector;
use crate::tls::extract_server_name;
use crate::wal::extract_seq;
use crate::wal::RawWalRecord;
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::info;
use tracing::warn;

pub struct ReplicationConsumer {
    pub stream_id: u32,
    pub endpoints: Vec<String>,
    pub tip: u64,
    pub tip_file: PathBuf,
    last_tip_persist: Instant,
    tip_persist_interval: Duration,
    tls_connector: TlsConnector,
}

/// How a single streamed connection ended — the signal
/// `run`'s reconnect loop acts on. `run_once` discards it.
enum StreamEnd {
    /// The peer closed the stream (EOF). `run` reconnects.
    Eof,
    /// The callback returned `false`. `run` stops.
    Stopped,
}

impl ReplicationConsumer {
    /// Create a consumer that tries `endpoints` in order on
    /// each connect attempt. The first endpoint that can
    /// serve the current tip wins; on a `ReplicationNotAvailable`
    /// reply the consumer closes that connection and tries
    /// the next endpoint with the same from_seq.
    pub fn new(
        stream_id: u32,
        endpoints: Vec<String>,
        tip_file: PathBuf,
        tls: TlsConfig,
    ) -> io::Result<Self> {
        if endpoints.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ReplicationConsumer requires at least one endpoint",
            ));
        }
        let tip = load_tip(&tip_file).unwrap_or(0);
        info!(
            "dxs consumer stream_id={} tip={} endpoints={:?}",
            stream_id, tip, endpoints
        );

        let client = tls.client.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "replication requires TLS: client CA cert missing",
            )
        })?;
        let tls_connector = build_connector(&client)?;

        Ok(Self {
            stream_id,
            endpoints,
            tip,
            tip_file,
            last_tip_persist: Instant::now(),
            tip_persist_interval: Duration::from_millis(10),
            tls_connector,
        })
    }

    /// Connect with reconnect + backoff, streaming live. The
    /// callback returns `true` to keep streaming, `false` to stop
    /// and return `Ok(())`. Reconnects on connection errors; only a
    /// `false` return (or the retry budget) ends it.
    pub async fn run<F>(&mut self, mut callback: F) -> io::Result<()>
    where
        F: FnMut(RawWalRecord) -> bool,
    {
        const BACKOFF_SECS: [u64; 5] = [1, 2, 4, 8, 30];
        const MAX_RETRIES: u32 = 20;

        let mut backoff_idx = 0usize;
        let mut consec_errors: u32 = 0;

        loop {
            match self.connect_and_stream(&mut callback).await {
                Ok(StreamEnd::Eof) => {
                    info!("stream ended, reconnecting");
                    backoff_idx = 0;
                    consec_errors = 0;
                }
                Ok(StreamEnd::Stopped) => {
                    info!("consumer callback requested stop");
                    return Ok(());
                }
                Err(e) => {
                    consec_errors += 1;
                    if consec_errors > MAX_RETRIES {
                        return Err(io::Error::other(format!(
                            "BLOCKED: {consec_errors} consecutive stream errors \
                             exhausted retry budget ({MAX_RETRIES}): {e}",
                        )));
                    }
                    let base_secs = BACKOFF_SECS[backoff_idx.min(BACKOFF_SECS.len() - 1)];
                    let sleep_ms = (base_secs as f64 * 1000.0 * jitter_factor()) as u64;
                    warn!(
                        "stream error ({}/{}): {}, retry in {}ms",
                        consec_errors, MAX_RETRIES, e, sleep_ms,
                    );
                    tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    if backoff_idx < BACKOFF_SECS.len() - 1 {
                        backoff_idx += 1;
                    }
                }
            }
        }
    }

    /// Connect once and stream until the connection ends
    /// or the callback returns `false`. No reconnect.
    pub async fn run_once<F>(&mut self, mut callback: F) -> io::Result<()>
    where
        F: FnMut(RawWalRecord) -> bool,
    {
        // No reconnect. Discard the stop/end distinction — it only
        // matters to `run`'s reconnect loop.
        self.connect_and_stream(&mut callback).await.map(|_| ())
    }

    /// Streams one connection (federating over endpoints). See
    /// [`StreamEnd`] for the two success outcomes.
    async fn connect_and_stream<F>(&mut self, callback: &mut F) -> io::Result<StreamEnd>
    where
        F: FnMut(RawWalRecord) -> bool,
    {
        let mut last_err: Option<io::Error> = None;
        for endpoint in &self.endpoints.clone() {
            let tcp_stream = match TcpStream::connect(endpoint).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("dxs: connect to {endpoint} failed: {e}");
                    last_err = Some(e);
                    continue;
                }
            };
            let result = match extract_server_name(endpoint) {
                Ok(server_name) => {
                    match self
                        .tls_connector
                        .clone()
                        .connect(server_name, tcp_stream)
                        .await
                    {
                        Ok(tls) => self.handle_stream(tls, callback).await,
                        Err(e) => Err(io::Error::other(format!("tls handshake failed: {e}"))),
                    }
                }
                Err(e) => Err(e),
            };
            match result {
                Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
                    warn!(
                        "dxs: {endpoint} cannot serve seq={}, trying next",
                        self.tip + 1
                    );
                    last_err = Some(io::Error::new(io::ErrorKind::NotFound, e.to_string()));
                    continue;
                }
                other => return other,
            }
        }
        Err(last_err.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "all dxs endpoints exhausted")
        }))
    }

    async fn handle_stream<S, F>(
        &mut self,
        mut stream: S,
        callback: &mut F,
    ) -> io::Result<StreamEnd>
    where
        S: AsyncReadExt + AsyncWriteExt + Unpin,
        F: FnMut(RawWalRecord) -> bool,
    {
        let req = ReplicationRequest {
            stream_id: self.stream_id,
            _pad0: 0,
            from_seq: self.tip + 1,
            _pad1: [0u8; 48],
        };
        let payload = as_bytes(&req);
        let crc = compute_crc32(payload);
        let hdr = WalHeader::new(RECORD_REPLICATION_REQUEST, payload.len() as u16, crc);
        stream.write_all(hdr.to_bytes()).await?;
        stream.write_all(payload).await?;

        let mut hdr_buf = [0u8; WalHeader::SIZE];
        // Stays `Eof` unless the callback returns false (`Stopped`).
        let mut end = StreamEnd::Eof;
        loop {
            match stream.read_exact(&mut hdr_buf).await {
                Ok(_) => {}
                Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(e) => return Err(e),
            }

            let header = WalHeader::from_bytes(&hdr_buf)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad header"))?;
            if header.record_type == RECORD_REPLICATION_NOT_AVAILABLE {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("replay not available from seq={}", self.tip + 1),
                ));
            }

            let payload_len = header.len as usize;
            let mut payload = vec![0u8; payload_len];
            stream.read_exact(&mut payload).await?;

            let computed = compute_crc32(&payload);
            if computed != header.crc32 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "crc mismatch"));
            }

            if let Some(seq) = extract_seq(&payload) {
                self.tip = self.tip.max(seq);
            }

            let keep_going = callback(RawWalRecord { header, payload });

            if self.last_tip_persist.elapsed() >= self.tip_persist_interval {
                persist_tip(&self.tip_file, self.tip)?;
                self.last_tip_persist = Instant::now();
            }

            if !keep_going {
                end = StreamEnd::Stopped;
                break;
            }
        }

        persist_tip(&self.tip_file, self.tip)?;
        Ok(end)
    }
}

fn jitter_factor() -> f64 {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(12345);
    0.8 + 0.4 * ((ns % 1000) as f64 / 1000.0)
}

/// The tip is stored as a JSON number — a bare decimal integer
/// (e.g. `42`), which is a valid JSON document — so the file is
/// human-readable and inspectable with `cat`. A missing, empty, or
/// unparseable file reads as tip 0 (replay from the start;
/// idempotent, so a format change just re-replays).
fn load_tip(path: &Path) -> io::Result<u64> {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    let tip = std::str::from_utf8(&data)
        .map(str::trim)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(tip)
}

fn persist_tip(path: &Path, tip: u64) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, tip.to_string().as_bytes())?;
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        let dir = File::open(parent)?;
        dir.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "replication_client_test.rs"]
mod replication_client_test;
