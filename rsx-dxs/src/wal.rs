use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Notify;
use tracing::debug;
use tracing::info;
use tracing::warn;

const MAX_PAYLOAD: u32 = 64 * 1024;

/// WalWriter: append-only WAL with buffered flush + rotation
pub struct WalWriter {
    pub stream_id: u32,
    pub next_seq: u64,
    buf: Vec<u8>,
    file: File,
    file_size: u64,
    first_seq: u64,
    last_seq: u64,
    wal_dir: PathBuf,
    max_file_size: u64,
    retention_ns: u64,
    listeners: Vec<Arc<Notify>>,
}

impl WalWriter {
    pub fn new(
        stream_id: u32,
        wal_dir: &Path,
        max_file_size: u64,
        retention_ns: u64,
    ) -> io::Result<Self> {
        let dir = wal_dir.join(stream_id.to_string());
        fs::create_dir_all(&dir)?;

        let active_path = dir.join(format!(
            "{}_active.wal", stream_id
        ));
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&active_path)?;

        let file_size = file.metadata()?.len();

        Ok(Self {
            stream_id,
            next_seq: 1,
            buf: Vec::with_capacity(64 * 1024),
            file,
            file_size,
            first_seq: 1,
            last_seq: 0,
            wal_dir: dir,
            max_file_size,
            retention_ns,
            listeners: Vec::new(),
        })
    }

    /// Append record to in-memory buffer. No I/O. O(1).
    /// Returns assigned seq.
    pub fn append(
        &mut self,
        record_type: u16,
        payload: &[u8],
    ) -> io::Result<u64> {
        let len = payload.len() as u32;
        if len > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload exceeds 64KB",
            ));
        }

        // backpressure: stall if buf > 2x max_file_size
        let limit = (self.max_file_size as usize)
            .saturating_mul(2)
            .max(256 * 1024);
        if self.buf.len() > limit {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "wal buffer full, backpressure",
            ));
        }

        let crc = compute_crc32(payload);
        let header = WalHeader::new(
            record_type, len, self.stream_id, crc,
        );

        self.buf.extend_from_slice(&header.to_bytes());
        self.buf.extend_from_slice(payload);

        let seq = self.next_seq;
        self.next_seq += 1;
        self.last_seq = seq;
        Ok(seq)
    }

    /// Flush buffer to disk with fsync.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }

        self.file.write_all(&self.buf)?;
        self.file.sync_all()?;
        self.file_size += self.buf.len() as u64;
        self.buf.clear();

        // notify live consumers
        for listener in &self.listeners {
            listener.notify_one();
        }

        // rotate if needed
        if self.file_size >= self.max_file_size {
            self.rotate()?;
        }

        Ok(())
    }

    /// Rotate: rename active -> seq range, open new active.
    fn rotate(&mut self) -> io::Result<()> {
        let active_path = self.wal_dir.join(format!(
            "{}_active.wal", self.stream_id
        ));
        let rotated_path = self.wal_dir.join(format!(
            "{}_{}_{}.wal",
            self.stream_id, self.first_seq, self.last_seq
        ));

        drop(std::mem::replace(
            &mut self.file,
            File::create("/dev/null")?,
        ));

        fs::rename(&active_path, &rotated_path)?;
        info!(
            "rotated wal {} -> {}",
            active_path.display(),
            rotated_path.display()
        );

        self.file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&active_path)?;
        self.file_size = 0;
        self.first_seq = self.next_seq;

        self.gc()?;
        Ok(())
    }

    /// Delete rotated files older than retention window.
    pub fn gc(&self) -> io::Result<()> {
        let entries = fs::read_dir(&self.wal_dir)?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if name.ends_with("_active.wal") {
                continue;
            }

            if let Some(info) = parse_wal_filename(&name) {
                if info.stream_id != self.stream_id {
                    continue;
                }
                let age_seqs = self.next_seq
                    .saturating_sub(info.last_seq);
                let retention_seqs =
                    (self.retention_ns / 1_000).max(1_000);
                if age_seqs > retention_seqs {
                    debug!(
                        "gc deleting {}",
                        entry.path().display()
                    );
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }

    pub fn add_listener(&mut self) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.listeners.push(notify.clone());
        notify
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }
}

/// Info parsed from a WAL filename
#[derive(Debug, Clone)]
pub struct WalFileInfo {
    pub path: PathBuf,
    pub stream_id: u32,
    pub first_seq: u64,
    pub last_seq: u64,
    pub is_active: bool,
}

fn parse_wal_filename(name: &str) -> Option<WalFileInfo> {
    let stem = name.strip_suffix(".wal")?;
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() != 3 {
        return None;
    }
    Some(WalFileInfo {
        path: PathBuf::new(),
        stream_id: parts[0].parse().ok()?,
        first_seq: parts[1].parse().ok()?,
        last_seq: parts[2].parse().ok()?,
        is_active: false,
    })
}

/// WalReader: sequential reading with CRC validation
pub struct WalReader {
    stream_id: u32,
    wal_dir: PathBuf,
    file: Option<File>,
    files: Vec<WalFileInfo>,
    file_idx: usize,
    header_buf: [u8; WalHeader::SIZE],
}

impl WalReader {
    /// Open reader starting at target_seq.
    pub fn open_from_seq(
        stream_id: u32,
        target_seq: u64,
        wal_dir: &Path,
    ) -> io::Result<Self> {
        let dir = wal_dir.join(stream_id.to_string());
        let mut files = list_wal_files(stream_id, &dir)?;
        files.sort_by_key(|f| f.first_seq);

        // start from first file if target_seq is 0
        let file_idx = if files.is_empty() {
            0
        } else if target_seq == 0 {
            0
        } else {
            // find file containing target_seq
            let mut idx = 0;
            for (i, f) in files.iter().enumerate() {
                if !f.is_active
                    && target_seq >= f.first_seq
                    && target_seq <= f.last_seq
                {
                    idx = i;
                    break;
                }
                // for active file or if target is beyond
                // all rotated, use last
                idx = i;
            }
            idx
        };

        let file = if file_idx < files.len() {
            Some(File::open(&files[file_idx].path)?)
        } else {
            None
        };

        Ok(Self {
            stream_id,
            wal_dir: dir,
            file,
            files,
            file_idx,
            header_buf: [0u8; WalHeader::SIZE],
        })
    }

    /// Read next record. Returns None at EOF.
    pub fn next(
        &mut self,
    ) -> io::Result<Option<RawWalRecord>> {
        loop {
            let file = match &mut self.file {
                Some(f) => f,
                None => return Ok(None),
            };

            match file.read_exact(&mut self.header_buf) {
                Ok(()) => {}
                Err(e)
                    if e.kind()
                        == io::ErrorKind::UnexpectedEof =>
                {
                    if !self.advance_file()? {
                        return Ok(None);
                    }
                    continue;
                }
                Err(e) => return Err(e),
            }

            let header =
                WalHeader::from_bytes(&self.header_buf)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "malformed header",
                        )
                    })?;

            if header.version != WalHeader::VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "unknown wal version {}",
                        header.version
                    ),
                ));
            }

            if header.len > MAX_PAYLOAD {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "payload length exceeds max",
                ));
            }

            let mut payload = vec![0u8; header.len as usize];
            match file.read_exact(&mut payload) {
                Ok(()) => {}
                Err(e)
                    if e.kind()
                        == io::ErrorKind::UnexpectedEof =>
                {
                    warn!(
                        "partial record at eof, truncating"
                    );
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }

            let computed = compute_crc32(&payload);
            if computed != header.crc32 {
                warn!(
                    "crc mismatch: computed={} stored={}",
                    computed, header.crc32
                );
                return Ok(None);
            }

            return Ok(Some(RawWalRecord {
                header,
                payload,
            }));
        }
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    pub fn wal_dir(&self) -> &Path {
        &self.wal_dir
    }

    fn advance_file(&mut self) -> io::Result<bool> {
        self.file_idx += 1;
        if self.file_idx < self.files.len() {
            self.file = Some(File::open(
                &self.files[self.file_idx].path,
            )?);
            return Ok(true);
        }
        self.file = None;
        Ok(false)
    }
}

/// Raw WAL record (header + payload bytes)
#[derive(Debug, Clone)]
pub struct RawWalRecord {
    pub header: WalHeader,
    pub payload: Vec<u8>,
}

fn list_wal_files(
    stream_id: u32,
    dir: &Path,
) -> io::Result<Vec<WalFileInfo>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.ends_with("_active.wal") {
            // include active file as last entry
            files.push(WalFileInfo {
                path: entry.path(),
                stream_id,
                first_seq: u64::MAX,
                last_seq: u64::MAX,
                is_active: true,
            });
            continue;
        }

        if let Some(mut info) = parse_wal_filename(&name_str) {
            if info.stream_id == stream_id {
                info.path = entry.path();
                files.push(info);
            }
        }
    }
    Ok(files)
}
