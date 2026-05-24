//! Replication — TCP catch-up server (the producer side).
//!
//! `ReplicationService` accepts TCP connections, parses a
//! `ReplicationRequest`, and either refuses with
//! `RECORD_REPLICATION_NOT_AVAILABLE` (when the requested
//! `from_seq` is below this node's oldest on-disk seq) or
//! streams WAL records in two phases:
//!
//! 1. Historical: drains the WAL from `from_seq` until the
//!    file tail.
//! 2. Live tail: subscribes to the `WalWriter` and forwards
//!    each new record as it lands on disk, until the client
//!    disconnects.
//!
//! The boundary record is `RECORD_CAUGHT_UP`. Optional TLS
//! per `TlsConfig` for cross-DC replication. See
//! `specs/10-replication.md`.

use crate::config::TlsConfig;
use crate::encode_utils::compute_crc32;
use crate::encode_utils::encode_record;
use crate::header::WalHeader;
use crate::protocol::CaughtUpRecord;
use crate::protocol::ReplicationNotAvailable;
use crate::protocol::ReplicationRequest;
use crate::protocol::RECORD_CAUGHT_UP;
use crate::protocol::RECORD_REPLICATION_NOT_AVAILABLE;
use crate::protocol::RECORD_REPLICATION_REQUEST;
use crate::tls::build_acceptor;
use crate::wal::WalReader;
use crate::wal::extract_seq;
use crate::wal::oldest_and_highest_seq;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::sleep;
use std::time::Duration;
use tokio_rustls::TlsAcceptor;
use tracing::error;
use tracing::info;
use tracing::warn;

fn time_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

#[derive(Clone)]
pub struct ReplicationService {
    pub wal_dir: PathBuf,
    pub tls_acceptor: Option<TlsAcceptor>,
}

impl ReplicationService {
    pub fn new(
        wal_dir: PathBuf,
        tls: Option<TlsConfig>,
    ) -> io::Result<Self> {
        let tls_acceptor = tls
            .and_then(|c| c.server)
            .map(|s| build_acceptor(&s))
            .transpose()?;

        Ok(Self {
            wal_dir,
            tls_acceptor,
        })
    }

    pub async fn serve(
        self,
        addr: SocketAddr,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr).await?;
        let tls_enabled = self.tls_acceptor.is_some();
        info!(
            "dxs replay server listening on {} (tls={})",
            addr, tls_enabled
        );
        let svc = Arc::new(self);
        loop {
            let (stream, peer) =
                listener.accept().await?;
            info!("dxs client connected from {}", peer);
            let svc = svc.clone();
            tokio::spawn(async move {
                let result = if let Some(ref acceptor) =
                    svc.tls_acceptor
                {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            handle_client(svc, tls_stream).await
                        }
                        Err(e) => Err(io::Error::other(
                            format!("tls handshake failed: {}", e),
                        )),
                    }
                } else {
                    handle_client(svc, stream).await
                };
                if let Err(e) = result {
                    warn!(
                        "dxs client {} error: {}",
                        peer, e
                    );
                }
            });
        }
    }
}

async fn handle_client<S>(
    svc: Arc<ReplicationService>,
    mut stream: S,
) -> io::Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let mut hdr_buf = [0u8; WalHeader::SIZE];
    stream.read_exact(&mut hdr_buf).await?;
    let hdr = WalHeader::from_bytes(&hdr_buf)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad header",
            )
        })?;
    if hdr.record_type != RECORD_REPLICATION_REQUEST {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected replay request",
        ));
    }
    let payload_len = hdr.len as usize;
    let mut payload_buf = vec![0u8; payload_len];
    stream.read_exact(&mut payload_buf).await?;

    let crc = compute_crc32(&payload_buf);
    if crc != hdr.crc32 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "replay request crc mismatch",
        ));
    }
    if payload_buf.len()
        < std::mem::size_of::<ReplicationRequest>()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "replay request too short",
        ));
    }
    let req = unsafe {
        std::ptr::read_unaligned(
            payload_buf.as_ptr()
                as *const ReplicationRequest,
        )
    };
    let stream_id = req.stream_id;
    let from_seq = req.from_seq;

    info!(
        "replay request stream_id={} from_seq={}",
        stream_id, from_seq
    );

    let range = oldest_and_highest_seq(
        stream_id, &svc.wal_dir, None,
    )?;
    let (my_oldest, my_highest) = range.unwrap_or((0, 0));
    let endpoint_can_serve = match range {
        Some((oldest, _)) => from_seq == 0 || from_seq >= oldest,
        None => from_seq == 0,
    };
    if !endpoint_can_serve {
        warn!(
            "replay refused stream_id={} from_seq={}              my_oldest={} my_highest={}",
            stream_id, from_seq, my_oldest, my_highest
        );
        let na = ReplicationNotAvailable {
            requested_from_seq: from_seq,
            my_oldest_seq: my_oldest,
            my_highest_seq: my_highest,
            stream_id,
            _pad: [0; 36],
        };
        let payload = unsafe {
            std::slice::from_raw_parts(
                &na as *const ReplicationNotAvailable
                    as *const u8,
                std::mem::size_of::<ReplicationNotAvailable>(),
            )
        };
        let encoded = encode_record(
            RECORD_REPLICATION_NOT_AVAILABLE,
            payload,
        );
        stream.write_all(&encoded).await?;
        stream.flush().await?;
        return Ok(());
    }

    let mut reader = WalReader::open_from_seq(
        stream_id,
        from_seq,
        &svc.wal_dir,
    )?;

    let mut last_seq = from_seq;
    loop {
        match reader.next() {
            Ok(Some(record)) => {
                let hdr_bytes =
                    record.header.to_bytes();
                stream
                    .write_all(&hdr_bytes)
                    .await?;
                stream
                    .write_all(&record.payload)
                    .await?;
                if let Some(seq) = extract_seq(&record.payload) {
                    last_seq = seq;
                }
            }
            Ok(None) => break,
            Err(e) => {
                warn!("wal read error: {}", e);
                return Err(e);
            }
        }
    }

    let caught_up = CaughtUpRecord {
        seq: 0,
        ts_ns: time_ns(),
        stream_id,
        _pad0: 0,
        live_seq: last_seq,
        _pad1: [0; 40],
    };
    let payload = unsafe {
        std::slice::from_raw_parts(
            &caught_up as *const CaughtUpRecord
                as *const u8,
            std::mem::size_of::<CaughtUpRecord>(),
        )
    };
    let encoded = encode_record(
        RECORD_CAUGHT_UP,
        payload,
    );
    stream.write_all(&encoded).await?;

    loop {
        sleep(Duration::from_millis(100)).await;

        let mut reader =
            match WalReader::open_from_seq(
                stream_id,
                last_seq + 1,
                &svc.wal_dir,
            ) {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        "wal open_from_seq failed                          stream_id={} seq={}: {}",
                        stream_id,
                        last_seq + 1,
                        e
                    );
                    continue;
                }
            };

        loop {
            match reader.next() {
                Ok(Some(record)) => {
                    let hdr_bytes =
                        record.header.to_bytes();
                    stream
                        .write_all(&hdr_bytes)
                        .await?;
                    stream
                        .write_all(&record.payload)
                        .await?;
                    if let Some(seq) = extract_seq(&record.payload) {
                        last_seq = seq;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    }
}
