use crate::config::TlsConfig;
use crate::encode_utils::encode_record;
use crate::header::WalHeader;
use crate::records::CaughtUpRecord;
use crate::records::ReplayRequest;
use crate::records::RECORD_CAUGHT_UP;
use crate::records::RECORD_REPLAY_REQUEST;
use crate::wal::WalReader;
use crate::wal::extract_seq;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::ServerConfig;
use rsx_types::time::time_ns;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tokio_rustls::TlsAcceptor;
use tracing::info;
use tracing::warn;

#[derive(Clone)]
pub struct DxsReplayService {
    pub wal_dir: PathBuf,
    pub listeners: Arc<
        RwLock<HashMap<u32, Vec<Arc<Notify>>>>,
    >,
    pub tls_acceptor: Option<TlsAcceptor>,
}

impl DxsReplayService {
    pub fn new(
        wal_dir: PathBuf,
        tls_config: Option<TlsConfig>,
    ) -> io::Result<Self> {
        let tls_acceptor = if let Some(cfg) = tls_config {
            if cfg.enabled {
                cfg.validate_server()?;
                Some(build_tls_acceptor(&cfg)?)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            wal_dir,
            listeners: Arc::new(RwLock::new(
                HashMap::new(),
            )),
            tls_acceptor,
        })
    }

    pub async fn add_listener(
        &self,
        stream_id: u32,
    ) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        let mut map = self.listeners.write().await;
        map.entry(stream_id)
            .or_default()
            .push(notify.clone());
        notify
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
                            handle_client_tls(
                                svc,
                                tls_stream,
                            )
                            .await
                        }
                        Err(e) => Err(io::Error::other(
                            format!("tls handshake failed: {}", e),
                        )),
                    }
                } else {
                    handle_client_plain(svc, stream).await
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

async fn handle_client_plain(
    svc: Arc<DxsReplayService>,
    stream: TcpStream,
) -> io::Result<()> {
    handle_client(svc, stream).await
}

async fn handle_client_tls(
    svc: Arc<DxsReplayService>,
    stream: tokio_rustls::server::TlsStream<
        TcpStream,
    >,
) -> io::Result<()> {
    handle_client(svc, stream).await
}

async fn handle_client<S>(
    svc: Arc<DxsReplayService>,
    mut stream: S,
) -> io::Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    // Read ReplayRequest: WalHeader(16) + payload(64)
    let mut hdr_buf = [0u8; WalHeader::SIZE];
    stream.read_exact(&mut hdr_buf).await?;
    let hdr = WalHeader::from_bytes(&hdr_buf)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad header",
            )
        })?;
    if hdr.record_type != RECORD_REPLAY_REQUEST {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected replay request",
        ));
    }
    let payload_len = hdr.len as usize;
    let mut payload_buf = vec![0u8; payload_len];
    stream.read_exact(&mut payload_buf).await?;

    if payload_buf.len()
        < std::mem::size_of::<ReplayRequest>()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "replay request too short",
        ));
    }
    let req = unsafe {
        std::ptr::read_unaligned(
            payload_buf.as_ptr()
                as *const ReplayRequest,
        )
    };
    let stream_id = req.stream_id;
    let from_seq = req.from_seq;

    info!(
        "replay request stream_id={} from_seq={}",
        stream_id, from_seq
    );

    let notify =
        svc.add_listener(stream_id).await;

    // Phase 1: historical replay
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

    // Send CaughtUp marker
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

    // Phase 2: live tail
    loop {
        notify.notified().await;

        let mut reader =
            match WalReader::open_from_seq(
                stream_id,
                last_seq + 1,
                &svc.wal_dir,
            ) {
                Ok(r) => r,
                Err(_) => continue,
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

fn build_tls_acceptor(
    cfg: &TlsConfig,
) -> io::Result<TlsAcceptor> {
    let cert_path =
        cfg.cert_path.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "cert_path required",
            )
        })?;
    let key_path =
        cfg.key_path.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "key_path required",
            )
        })?;

    let cert_pem = fs::read(cert_path)?;
    let key_pem = fs::read(key_path)?;

    let certs = load_certs(&cert_pem)?;
    let key = load_private_key(&key_pem)?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("tls config error: {}", e),
            )
        })?;

    Ok(TlsAcceptor::from(Arc::new(config)))
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

fn load_private_key(
    pem: &[u8],
) -> io::Result<PrivateKeyDer<'static>> {
    let mut cursor = io::Cursor::new(pem);
    let keys =
        rustls_pemfile::private_key(&mut cursor)
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad key pem: {}", e),
                )
            })?;

    keys.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "no private key found",
        )
    })
}
