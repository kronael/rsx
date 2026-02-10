use crate::encode_utils::encode_record;
use crate::header::WalHeader;
use crate::records::CaughtUpRecord;
use crate::records::PayloadPreamble;
use crate::records::ReplayRequest;
use crate::records::RECORD_CAUGHT_UP;
use crate::records::RECORD_REPLAY_REQUEST;
use crate::wal::WalReader;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tracing::info;
use tracing::warn;

pub struct DxsReplayService {
    pub wal_dir: PathBuf,
    pub listeners: Arc<
        RwLock<HashMap<u32, Vec<Arc<Notify>>>>,
    >,
}

impl DxsReplayService {
    pub fn new(wal_dir: PathBuf) -> Self {
        Self {
            wal_dir,
            listeners: Arc::new(RwLock::new(
                HashMap::new(),
            )),
        }
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
        info!("dxs replay server listening on {}", addr);
        let svc = Arc::new(self);
        loop {
            let (stream, peer) =
                listener.accept().await?;
            info!("dxs client connected from {}", peer);
            let svc = svc.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    handle_client(svc, stream).await
                {
                    warn!(
                        "dxs client {} error: {}",
                        peer, e
                    );
                }
            });
        }
    }
}

async fn handle_client(
    svc: Arc<DxsReplayService>,
    mut stream: TcpStream,
) -> std::io::Result<()> {
    // Read ReplayRequest: WalHeader(16) + payload(64)
    let mut hdr_buf = [0u8; WalHeader::SIZE];
    stream.read_exact(&mut hdr_buf).await?;
    let preamble = WalHeader::from_bytes(&hdr_buf)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad header",
            )
        })?;
    if preamble.record_type != RECORD_REPLAY_REQUEST {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected replay request",
        ));
    }
    let payload_len = preamble.len as usize;
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
        std::ptr::read(
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
                last_seq = last_seq.max(from_seq);
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
        preamble: PayloadPreamble {
            seq: 0,
            ver: 1,
            kind: 0,
            _pad0: 0,
            len: std::mem::size_of::<CaughtUpRecord>()
                as u32,
        },
        ts_ns: std::time::SystemTime::now()
            .duration_since(
                std::time::UNIX_EPOCH,
            )
            .unwrap_or_default()
            .as_nanos() as u64,
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
        stream_id,
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
                    last_seq += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    }
}
