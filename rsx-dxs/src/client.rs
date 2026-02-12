use crate::config::TlsConfig;
use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::ReplayRequest;
use crate::records::RECORD_BBO;
use crate::records::RECORD_CANCEL_REQUEST;
use crate::records::RECORD_CAUGHT_UP;
use crate::records::RECORD_CONFIG_APPLIED;
use crate::records::RECORD_FILL;
use crate::records::RECORD_HEARTBEAT;
use crate::records::RECORD_MARK_PRICE;
use crate::records::RECORD_NAK;
use crate::records::RECORD_ORDER_ACCEPTED;
use crate::records::RECORD_ORDER_CANCELLED;
use crate::records::RECORD_ORDER_DONE;
use crate::records::RECORD_ORDER_FAILED;
use crate::records::RECORD_ORDER_INSERTED;
use crate::records::RECORD_ORDER_REQUEST;
use crate::records::RECORD_ORDER_RESPONSE;
use crate::records::RECORD_REPLAY_REQUEST;
use crate::records::RECORD_STATUS_MESSAGE;
use crate::wal::RawWalRecord;
use crate::wal::extract_seq;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::ServerName;
use rustls::ClientConfig;
use rustls::RootCertStore;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::info;
use tracing::warn;

fn is_known_record_type(record_type: u16) -> bool {
    matches!(
        record_type,
        RECORD_FILL
            | RECORD_BBO
            | RECORD_ORDER_INSERTED
            | RECORD_ORDER_CANCELLED
            | RECORD_ORDER_DONE
            | RECORD_CONFIG_APPLIED
            | RECORD_CAUGHT_UP
            | RECORD_ORDER_ACCEPTED
            | RECORD_MARK_PRICE
            | RECORD_ORDER_REQUEST
            | RECORD_ORDER_RESPONSE
            | RECORD_CANCEL_REQUEST
            | RECORD_ORDER_FAILED
            | RECORD_STATUS_MESSAGE
            | RECORD_NAK
            | RECORD_HEARTBEAT
            | RECORD_REPLAY_REQUEST
    )
}

pub struct DxsConsumer {
    pub stream_id: u32,
    pub producer_addr: String,
    pub tip: u64,
    pub tip_file: PathBuf,
    last_tip_persist: Instant,
    tip_persist_interval: Duration,
    tls_connector: Option<TlsConnector>,
    server_name: Option<ServerName<'static>>,
}

impl DxsConsumer {
    pub fn new(
        stream_id: u32,
        producer_addr: String,
        tip_file: PathBuf,
        tls_config: Option<TlsConfig>,
    ) -> io::Result<Self> {
        let tip = load_tip(&tip_file).unwrap_or(0);
        info!(
            "dxs consumer stream_id={} tip={} addr={}",
            stream_id, tip, producer_addr
        );

        let (tls_connector, server_name) = if let Some(cfg) = tls_config {
            if cfg.enabled {
                cfg.validate_client()?;
                let connector = build_tls_connector(&cfg)?;
                let name = extract_server_name(&producer_addr)?;
                (Some(connector), Some(name))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Ok(Self {
            stream_id,
            producer_addr,
            tip,
            tip_file,
            last_tip_persist: Instant::now(),
            tip_persist_interval: Duration::from_millis(10),
            tls_connector,
            server_name,
        })
    }

    pub async fn run<F>(
        &mut self,
        mut callback: F,
    ) -> io::Result<()>
    where
        F: FnMut(RawWalRecord),
    {
        let backoff_schedule = [1, 2, 4, 8, 30];
        let mut backoff_idx = 0;

        loop {
            match self
                .connect_and_stream(&mut callback)
                .await
            {
                Ok(()) => {
                    info!("stream ended, reconnecting");
                    backoff_idx = 0;
                }
                Err(e) => {
                    let secs = backoff_schedule
                        [backoff_idx.min(
                            backoff_schedule.len() - 1,
                        )];
                    warn!(
                        "stream error: {}, retrying in {}s",
                        e, secs
                    );
                    tokio::time::sleep(
                        Duration::from_secs(secs),
                    )
                    .await;
                    if backoff_idx
                        < backoff_schedule.len() - 1
                    {
                        backoff_idx += 1;
                    }
                }
            }
        }
    }

    /// Connect once and stream until the connection ends.
    /// Unlike `run`, does not reconnect -- returns when
    /// the stream closes or the callback returns `false`.
    pub fn run_once<F>(
        &mut self,
        mut callback: F,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = io::Result<()>> + '_>,
    >
    where
        F: FnMut(RawWalRecord) -> bool + 'static,
    {
        Box::pin(async move {
            self.connect_and_stream_stoppable(
                &mut callback,
            )
            .await
        })
    }

    async fn connect_and_stream_stoppable<F>(
        &mut self,
        callback: &mut F,
    ) -> io::Result<()>
    where
        F: FnMut(RawWalRecord) -> bool,
    {
        let tcp_stream =
            TcpStream::connect(&self.producer_addr)
                .await?;

        if let (Some(connector), Some(server_name)) =
            (&self.tls_connector, &self.server_name)
        {
            let tls_stream = connector
                .connect(server_name.clone(), tcp_stream)
                .await
                .map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!(
                            "tls handshake failed: {}",
                            e
                        ),
                    )
                })?;
            self.handle_stream_stoppable(
                tls_stream, callback,
            )
            .await
        } else {
            self.handle_stream_stoppable(
                tcp_stream, callback,
            )
            .await
        }
    }

    async fn connect_and_stream<F>(
        &mut self,
        callback: &mut F,
    ) -> io::Result<()>
    where
        F: FnMut(RawWalRecord),
    {
        let tcp_stream =
            TcpStream::connect(&self.producer_addr)
                .await?;

        if let (Some(connector), Some(server_name)) =
            (&self.tls_connector, &self.server_name)
        {
            let tls_stream = connector
                .connect(server_name.clone(), tcp_stream)
                .await
                .map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("tls handshake failed: {}", e),
                    )
                })?;
            self.handle_stream(tls_stream, callback).await
        } else {
            self.handle_stream(tcp_stream, callback).await
        }
    }

    async fn handle_stream<S>(
        &mut self,
        mut stream: S,
        callback: &mut impl FnMut(RawWalRecord),
    ) -> io::Result<()>
    where
        S: AsyncReadExt + AsyncWriteExt + Unpin,
    {
        let req = ReplayRequest {
            stream_id: self.stream_id,
            _pad0: 0,
            from_seq: self.tip + 1,
            _pad1: [0u8; 48],
        };
        let req_size =
            std::mem::size_of::<ReplayRequest>();
        let payload = unsafe {
            std::slice::from_raw_parts(
                &req as *const ReplayRequest
                    as *const u8,
                req_size,
            )
        };
        let crc = compute_crc32(payload);
        let hdr = WalHeader::new(
            RECORD_REPLAY_REQUEST,
            payload.len() as u16,
            crc,
        );
        stream.write_all(&hdr.to_bytes()).await?;
        stream.write_all(payload).await?;

        let mut hdr_buf = [0u8; WalHeader::SIZE];
        loop {
            match stream.read_exact(&mut hdr_buf).await {
                Ok(_) => {}
                Err(ref e)
                    if e.kind()
                        == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }

            let header =
                WalHeader::from_bytes(&hdr_buf)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "bad header",
                        )
                    })?;

            let payload_len = header.len as usize;
            let mut payload = vec![0u8; payload_len];
            stream.read_exact(&mut payload).await?;

            let computed = compute_crc32(&payload);
            if computed != header.crc32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "crc mismatch",
                ));
            }

            // Advance tip from record seq when available.
            // Fallback to +1 for records without seq.
            let next_tip = extract_seq(&payload)
                .unwrap_or_else(|| self.tip.saturating_add(1));
            self.tip = self.tip.max(next_tip);

            if !is_known_record_type(
                header.record_type,
            ) {
                warn!(
                    "unknown record type {}, \
                     skipping",
                    header.record_type
                );
                continue;
            }

            let record =
                RawWalRecord { header, payload };
            callback(record);

            if self.last_tip_persist.elapsed()
                >= self.tip_persist_interval
            {
                persist_tip(&self.tip_file, self.tip)?;
                self.last_tip_persist = Instant::now();
            }
        }

        persist_tip(&self.tip_file, self.tip)?;
        Ok(())
    }

    async fn handle_stream_stoppable<S, F>(
        &mut self,
        mut stream: S,
        callback: &mut F,
    ) -> io::Result<()>
    where
        S: AsyncReadExt + AsyncWriteExt + Unpin,
        F: FnMut(RawWalRecord) -> bool,
    {
        let req = ReplayRequest {
            stream_id: self.stream_id,
            _pad0: 0,
            from_seq: self.tip + 1,
            _pad1: [0u8; 48],
        };
        let req_size =
            std::mem::size_of::<ReplayRequest>();
        let payload = unsafe {
            std::slice::from_raw_parts(
                &req as *const ReplayRequest
                    as *const u8,
                req_size,
            )
        };
        let crc = compute_crc32(payload);
        let hdr = WalHeader::new(
            RECORD_REPLAY_REQUEST,
            payload.len() as u16,
            crc,
        );
        stream.write_all(&hdr.to_bytes()).await?;
        stream.write_all(payload).await?;

        let mut hdr_buf = [0u8; WalHeader::SIZE];
        loop {
            match stream.read_exact(&mut hdr_buf).await {
                Ok(_) => {}
                Err(ref e)
                    if e.kind()
                        == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }

            let header =
                WalHeader::from_bytes(&hdr_buf)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "bad header",
                        )
                    })?;

            let payload_len = header.len as usize;
            let mut payload = vec![0u8; payload_len];
            stream.read_exact(&mut payload).await?;

            let computed = compute_crc32(&payload);
            if computed != header.crc32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "crc mismatch",
                ));
            }

            let next_tip = extract_seq(&payload)
                .unwrap_or_else(|| {
                    self.tip.saturating_add(1)
                });
            self.tip = self.tip.max(next_tip);

            if !is_known_record_type(
                header.record_type,
            ) {
                warn!(
                    "unknown record type {}, \
                     skipping",
                    header.record_type
                );
                continue;
            }

            let record =
                RawWalRecord { header, payload };
            let keep_going = callback(record);

            if self.last_tip_persist.elapsed()
                >= self.tip_persist_interval
            {
                persist_tip(&self.tip_file, self.tip)?;
                self.last_tip_persist = Instant::now();
            }

            if !keep_going {
                break;
            }
        }

        persist_tip(&self.tip_file, self.tip)?;
        Ok(())
    }
}

fn load_tip(path: &Path) -> io::Result<u64> {
    let data = fs::read(path)?;
    if data.len() < 8 {
        return Ok(0);
    }
    let bytes: [u8; 8] =
        data[..8].try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "bad tip file",
            )
        })?;
    Ok(u64::from_le_bytes(bytes))
}

fn persist_tip(path: &Path, tip: u64) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, tip.to_le_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn build_tls_connector(
    cfg: &TlsConfig,
) -> io::Result<TlsConnector> {
    let mut root_store = RootCertStore::empty();

    if let Some(ca_path) = &cfg.cert_path {
        let ca_pem = fs::read(ca_path)?;
        let certs = load_certs(&ca_pem)?;
        for cert in certs {
            root_store.add(cert).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("failed to add cert: {}", e),
                )
            })?;
        }
    } else {
        // TODO: Add webpki_roots once cargo registry is accessible
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "TLS requires cert_path in config",
        ));
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

fn load_certs(
    pem: &[u8],
) -> io::Result<Vec<CertificateDer<'static>>> {
    let mut cursor = io::Cursor::new(pem);
    rustls_pemfile::certs(&mut cursor)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("bad cert pem: {}", e),
            )
        })
}

fn extract_server_name(
    addr: &str,
) -> io::Result<ServerName<'static>> {
    let host = addr
        .split(':')
        .next()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid address format",
            )
        })?
        .to_string();

    ServerName::try_from(host)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid server name: {}", e),
            )
        })
        .map(|name| name.to_owned())
}
