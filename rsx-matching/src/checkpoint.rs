use apache_avro::from_value;
use apache_avro::Reader;
use apache_avro::Schema;
use apache_avro::Writer;
use rsx_book::book::Orderbook;
use rsx_book::snapshot;
use rsx_book::snapshot::BookSnapshotV1;
use rsx_book::snapshot::SnapshotU64;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io;
use std::io::Read as _;
use std::io::Seek as _;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;
use tracing::info;
use tracing::warn;

const LEGACY_CHECKPOINT_MAGIC: u32 = 0x5253_5843;
const LEGACY_CHECKPOINT_VERSION: u32 = 1;
const CHECKPOINT_SCHEMA: &str = include_str!("../checkpoint-v1.avsc");
const CHECKPOINT_SCHEMA_VERSION: i32 = 1;
const AVRO_MAGIC: [u8; 4] = *b"Obj\x01";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchingCheckpointV1 {
    pub schema_version: i32,
    pub wal_seq: SnapshotU64,
    pub book: BookSnapshotV1,
}

impl MatchingCheckpointV1 {
    pub fn new(wal_seq: u64, book: BookSnapshotV1) -> Self {
        Self {
            schema_version: CHECKPOINT_SCHEMA_VERSION,
            wal_seq: SnapshotU64::new(wal_seq),
            book,
        }
    }

    /// Capture `book` as of WAL seq `wal_seq`. Fails while a migration is in
    /// flight.
    pub fn capture(book: &Orderbook, wal_seq: u64) -> io::Result<Self> {
        Ok(Self::new(wal_seq, snapshot::capture(book)?))
    }
}

enum CheckpointCommand {
    Persist(MatchingCheckpointV1),
    Shutdown,
}

pub struct CheckpointWriter {
    sender: mpsc::SyncSender<CheckpointCommand>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl CheckpointWriter {
    pub fn start(wal_dir: &str, symbol_id: u32) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let wal_dir = wal_dir.to_owned();
        let worker = thread::Builder::new()
            .name(format!("rsx-me-checkpoint-{symbol_id}"))
            .spawn(move || run_checkpoint_worker(receiver, wal_dir, symbol_id))?;
        Ok(Self {
            sender,
            worker: Some(worker),
        })
    }

    pub fn submit(&self, checkpoint: MatchingCheckpointV1) -> io::Result<()> {
        match self.sender.try_send(CheckpointCommand::Persist(checkpoint)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "checkpoint writer is still busy",
            )),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "checkpoint writer stopped",
            )),
        }
    }

    pub fn shutdown(mut self, final_checkpoint: Option<MatchingCheckpointV1>) -> io::Result<()> {
        let mut send_error = None;
        if let Some(checkpoint) = final_checkpoint {
            if self
                .sender
                .send(CheckpointCommand::Persist(checkpoint))
                .is_err()
            {
                send_error = Some(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "checkpoint writer stopped before final checkpoint",
                ));
            }
        }
        if self.sender.send(CheckpointCommand::Shutdown).is_err() && send_error.is_none() {
            send_error = Some(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "checkpoint writer stopped before shutdown",
            ));
        }
        drop(self.sender);

        let worker = self
            .worker
            .take()
            .ok_or_else(|| io::Error::other("checkpoint worker missing"))?;
        let result = worker
            .join()
            .map_err(|_| io::Error::other("checkpoint worker panicked"))?;
        result?;
        if let Some(error) = send_error {
            return Err(error);
        }
        Ok(())
    }
}

fn run_checkpoint_worker(
    receiver: mpsc::Receiver<CheckpointCommand>,
    wal_dir: String,
    symbol_id: u32,
) -> io::Result<()> {
    while let Ok(command) = receiver.recv() {
        match command {
            CheckpointCommand::Persist(checkpoint) => {
                persist_checkpoint(&checkpoint, &wal_dir, symbol_id)?;
            }
            CheckpointCommand::Shutdown => return Ok(()),
        }
    }
    Ok(())
}

fn checkpoint_path(wal_dir: &str, symbol_id: u32) -> PathBuf {
    PathBuf::from(wal_dir)
        .join(symbol_id.to_string())
        .join("snapshot.bin")
}

fn avro_error(error: apache_avro::Error) -> io::Error {
    io::Error::other(error)
}

fn read_avro_checkpoint(file: fs::File) -> io::Result<MatchingCheckpointV1> {
    let mut reader = Reader::new(file).map_err(avro_error)?;
    let value = reader
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty checkpoint"))?
        .map_err(avro_error)?;
    let checkpoint: MatchingCheckpointV1 = from_value(&value).map_err(avro_error)?;
    if checkpoint.schema_version != CHECKPOINT_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported checkpoint schema version",
        ));
    }
    if let Some(value) = reader.next() {
        value.map_err(avro_error)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "checkpoint contains multiple records",
        ));
    }
    Ok(checkpoint)
}

fn persist_checkpoint(
    checkpoint: &MatchingCheckpointV1,
    wal_dir: &str,
    symbol_id: u32,
) -> io::Result<()> {
    let dir = PathBuf::from(wal_dir).join(symbol_id.to_string());
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("snapshot.bin.tmp");
    let dest = dir.join("snapshot.bin");
    let schema = Schema::parse_str(CHECKPOINT_SCHEMA).map_err(avro_error)?;
    let file = fs::File::create(&tmp)?;
    let mut writer = Writer::new(&schema, file);
    writer.append_ser(checkpoint).map_err(avro_error)?;
    let file = writer.into_inner().map_err(avro_error)?;
    file.sync_all()?;
    fs::rename(&tmp, &dest)?;
    fs::File::open(&dir)?.sync_all()?;
    Ok(())
}

/// Load an Avro checkpoint or a legacy v1 envelope with RXSN v2/v3 state.
pub fn load_checkpoint(wal_dir: &str, symbol_id: u32) -> Option<(Box<Orderbook>, u64)> {
    let path = checkpoint_path(wal_dir, symbol_id);
    let mut file = fs::File::open(&path).ok()?;
    let mut magic = [0u8; 4];
    if let Err(error) = file.read_exact(&mut magic) {
        warn!("checkpoint header load failed: {error}");
        return None;
    }
    if let Err(error) = file.rewind() {
        warn!("checkpoint rewind failed: {error}");
        return None;
    }

    let loaded = if magic == AVRO_MAGIC {
        read_avro_checkpoint(file).and_then(|checkpoint| {
            let wal_seq = checkpoint.wal_seq.get();
            let book = snapshot::restore(checkpoint.book)?;
            Ok((book, wal_seq))
        })
    } else {
        let mut header = [0u8; 16];
        file.read_exact(&mut header).and_then(|()| {
            let header_magic = u32::from_le_bytes(header[0..4].try_into().expect("4-byte slice"));
            let version = u32::from_le_bytes(header[4..8].try_into().expect("4-byte slice"));
            if header_magic != LEGACY_CHECKPOINT_MAGIC || version != LEGACY_CHECKPOINT_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported legacy checkpoint header",
                ));
            }
            let wal_seq = u64::from_le_bytes(header[8..16].try_into().expect("8-byte slice"));
            snapshot::load(&mut file).map(|book| (book, wal_seq))
        })
    };

    match loaded {
        Ok((book, wal_seq)) => {
            info!(wal_seq, "loaded checkpoint from {}", path.display());
            Some((book, wal_seq))
        }
        Err(error) => {
            warn!("checkpoint load failed: {error}, starting empty");
            None
        }
    }
}

/// Capture and synchronously persist one Avro checkpoint.
pub fn save_checkpoint(
    book: &Orderbook,
    wal_dir: &str,
    symbol_id: u32,
    wal_last_seq: u64,
) -> io::Result<()> {
    let checkpoint = MatchingCheckpointV1::capture(book, wal_last_seq)?;
    persist_checkpoint(&checkpoint, wal_dir, symbol_id)
}
