use crate::proto::dxs_replay_server::DxsReplay;
use crate::proto::dxs_replay_server::DxsReplayServer;
use crate::proto::ReplayRequest;
use crate::proto::WalBytes;
use crate::records::CaughtUpRecord;
use crate::records::RECORD_CAUGHT_UP;
use crate::wal::WalReader;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tonic::Request;
use tonic::Response;
use tonic::Status;
use tracing::info;

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
            listeners: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a flush listener for a stream_id.
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

    pub fn into_service(self) -> DxsReplayServer<Self> {
        DxsReplayServer::new(self)
    }

    pub async fn serve(
        self,
        addr: SocketAddr,
    ) -> Result<(), tonic::transport::Error> {
        info!("dxs replay server listening on {}", addr);
        tonic::transport::Server::builder()
            .add_service(self.into_service())
            .serve(addr)
            .await
    }
}

#[tonic::async_trait]
impl DxsReplay for DxsReplayService {
    type StreamStream = tokio_stream::wrappers::ReceiverStream<
        Result<WalBytes, Status>,
    >;

    async fn stream(
        &self,
        request: Request<ReplayRequest>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        let req = request.into_inner();
        let stream_id = req.stream_id;
        let from_seq = req.from_seq;

        info!(
            "replay request stream_id={} from_seq={}",
            stream_id, from_seq
        );

        let wal_dir = self.wal_dir.clone();
        let notify = self.add_listener(stream_id).await;

        let (tx, rx) = tokio::sync::mpsc::channel(256);

        tokio::spawn(async move {
            // phase 1: historical replay
            let mut reader = match WalReader::open_from_seq(
                stream_id,
                from_seq,
                &wal_dir,
            ) {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx
                        .send(Err(Status::internal(format!(
                            "open wal: {}", e
                        ))))
                        .await;
                    return;
                }
            };

            let mut last_seq = from_seq;
            loop {
                match reader.next() {
                    Ok(Some(record)) => {
                        let mut bytes = Vec::with_capacity(
                            crate::header::WalHeader::SIZE
                                + record.payload.len(),
                        );
                        bytes.extend_from_slice(
                            &record.header.to_bytes(),
                        );
                        bytes.extend_from_slice(&record.payload);
                        last_seq = last_seq.max(from_seq);
                        if tx
                            .send(Ok(WalBytes { record: bytes }))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx
                            .send(Err(Status::internal(format!(
                                "read wal: {}", e
                            ))))
                            .await;
                        return;
                    }
                }
            }

            // send CaughtUp marker
            let caught_up = CaughtUpRecord {
                seq: 0,
                ts_ns: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
                stream_id,
                live_seq: last_seq,
                _pad1: [0; 36],
            };
            let payload = unsafe {
                std::slice::from_raw_parts(
                    &caught_up as *const CaughtUpRecord
                        as *const u8,
                    std::mem::size_of::<CaughtUpRecord>(),
                )
            };
            let encoded = crate::encode_utils::encode_record(
                RECORD_CAUGHT_UP,
                stream_id,
                payload,
            );
            if tx
                .send(Ok(WalBytes { record: encoded }))
                .await
                .is_err()
            {
                return;
            }

            // phase 2: live tail
            loop {
                notify.notified().await;

                // re-open reader from where we left off
                let mut reader = match WalReader::open_from_seq(
                    stream_id,
                    last_seq + 1,
                    &wal_dir,
                ) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                loop {
                    match reader.next() {
                        Ok(Some(record)) => {
                            let mut bytes = Vec::with_capacity(
                                crate::header::WalHeader::SIZE
                                    + record.payload.len(),
                            );
                            bytes.extend_from_slice(
                                &record.header.to_bytes(),
                            );
                            bytes.extend_from_slice(
                                &record.payload,
                            );
                            last_seq += 1;
                            if tx
                                .send(Ok(WalBytes {
                                    record: bytes,
                                }))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            }
        });

        Ok(Response::new(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
    }
}
