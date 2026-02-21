use crate::encode_utils::as_bytes;
use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::CmpRecord;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::Notify;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

const MAX_PAYLOAD: u16 = 65535;

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
    archive_dir: Option<PathBuf>,
    max_file_size: u64,
    retention_ns: u64,
    listeners: Vec<Arc<Notify>>,
    flush_stalled: bool,
    records_since_flush: u32,
}

impl WalWriter {
    pub fn new(
        stream_id: u32,
        wal_dir: &Path,
        archive_dir: Option<PathBuf>,
        max_file_size: u64,
        retention_ns: u64,
    ) -> io::Result<Self> {
        let dir = wal_dir.join(stream_id.to_string());
        fs::create_dir_all(&dir)?;

        if let Some(ref archive) = archive_dir {
            let archive_stream_dir = archive.join(stream_id.to_string());
            fs::create_dir_all(&archive_stream_dir)?;
        }

        let active_path = dir.join(format!(
            "{}_active.wal", stream_id
        ));
        let file = OpenOptions::new()
            .create(true)
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
            archive_dir,
            max_file_size,
            retention_ns,
            listeners: Vec::new(),
            flush_stalled: false,
            records_since_flush: 0,
        })
    }

    /// Append typed CMP record to in-memory buffer.
    /// Assigns seq via CmpRecord::set_seq. No I/O. O(1).
    /// Returns assigned seq.
    pub fn append<T: CmpRecord>(
        &mut self,
        record: &mut T,
    ) -> io::Result<u64> {
        let payload_len = std::mem::size_of::<T>();
        if payload_len > MAX_PAYLOAD as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload exceeds 64KB",
            ));
        }

        if self.flush_stalled {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "flush stalled, backpressure",
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

        let seq = self.next_seq;
        self.next_seq += 1;
        self.last_seq = seq;

        record.set_seq(seq);

        let payload = as_bytes(record);
        let crc = compute_crc32(payload);
        let header = WalHeader::new(
            T::record_type(),
            payload_len as u16,
            crc,
        );

        self.buf.extend_from_slice(&header.to_bytes());
        self.buf.extend_from_slice(payload);
        self.records_since_flush += 1;
        Ok(seq)
    }

    /// Flush buffer to disk with fsync.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        // Reset stall at start of each flush cycle so a
        // previous slow flush doesn't block the next batch.
        self.flush_stalled = false;

        // rotate before writing if current file + buffer would exceed max
        if self.file_size > 0
            && self.file_size + self.buf.len() as u64
                >= self.max_file_size
        {
            self.rotate()?;
        }

        self.file.write_all(&self.buf)?;
        let t0 = Instant::now();
        self.file.sync_all()?;
        let elapsed = t0.elapsed();
        self.file_size += self.buf.len() as u64;
        self.buf.clear();
        self.records_since_flush = 0;

        if elapsed > Duration::from_millis(10) {
            warn!("flush took {}ms", elapsed.as_millis());
            self.flush_stalled = true;
        } else {
            self.flush_stalled = false;
        }

        // notify live consumers
        for listener in &self.listeners {
            listener.notify_one();
        }

        // rotate after write if file reached max
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

        if let Err(e) = self.file.sync_all() {
            error!(
                "sync_all failed before wal rotate: {}",
                e
            );
            return Err(e);
        }
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
            .append(true)
            .open(&active_path)?;
        self.file_size = 0;
        self.first_seq = self.next_seq;

        self.gc()?;
        Ok(())
    }

    /// Delete rotated files older than retention window.
    /// If archive_dir configured, move to archive first.
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
                let meta = match std::fs::metadata(
                    entry.path(),
                ) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let age = std::time::SystemTime::now()
                    .duration_since(
                        meta.modified()
                            .unwrap_or(
                                std::time::SystemTime::now()
                            ),
                    )
                    .unwrap_or_default();
                if age.as_nanos() as u64
                    > self.retention_ns
                {
                    if let Some(ref archive) = self.archive_dir {
                        self.archive_file(&entry.path(), archive)?;
                    } else {
                        debug!(
                            "gc deleting {}",
                            entry.path().display()
                        );
                        fs::remove_file(entry.path())?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Move WAL file to archive directory.
    fn archive_file(
        &self,
        source: &Path,
        archive_dir: &Path,
    ) -> io::Result<()> {
        let filename = source.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "no filename",
            )
        })?;
        let dest = archive_dir
            .join(self.stream_id.to_string())
            .join(filename);

        debug!(
            "archiving {} -> {}",
            source.display(),
            dest.display()
        );

        fs::rename(source, &dest)?;
        info!("archived to {}", dest.display());
        Ok(())
    }

    pub fn add_listener(&mut self) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.listeners.push(notify.clone());
        notify
    }

    pub fn should_flush(&self) -> bool {
        self.records_since_flush >= 1000
    }

    pub fn flush_stalled(&self) -> bool {
        self.flush_stalled
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
    hot_wal_dir: PathBuf,
    archive_dir: Option<PathBuf>,
    file: Option<File>,
    files: Vec<WalFileInfo>,
    file_idx: usize,
    header_buf: [u8; WalHeader::SIZE],
    in_archive: bool,
}

impl WalReader {
    /// Open reader starting at target_seq.
    pub fn open_from_seq(
        stream_id: u32,
        target_seq: u64,
        wal_dir: &Path,
    ) -> io::Result<Self> {
        Self::open_from_seq_with_archive(
            stream_id, target_seq, wal_dir, None,
        )
    }

    /// Open reader with archive fallback support.
    pub fn open_from_seq_with_archive(
        stream_id: u32,
        target_seq: u64,
        wal_dir: &Path,
        archive_dir: Option<&Path>,
    ) -> io::Result<Self> {
        let hot_dir = wal_dir.join(stream_id.to_string());
        let mut files = list_wal_files(stream_id, &hot_dir)?;
        files.sort_by_key(|f| f.first_seq);

        // check if target_seq is in hot WAL range
        let in_hot_range = if files.is_empty() {
            // if hot WAL is empty and we have archive,
            // fallback to archive (in_hot_range = false)
            // if no archive, treat as in_hot_range (will
            // return empty)
            archive_dir.is_none()
        } else {
            // find first non-active file's first_seq
            let first_available = files
                .iter()
                .find(|f| !f.is_active)
                .map(|f| f.first_seq)
                .unwrap_or(u64::MAX);

            // if target_seq is 0 (start from beginning) and we
            // have an archive, check archive first
            if target_seq == 0 && archive_dir.is_some() {
                false // fallback to archive
            } else {
                target_seq >= first_available
                    || (target_seq == 0
                        && first_available == u64::MAX)
            }
        };

        // if not in hot range, try archive fallback
        let (files, current_dir, in_archive) =
            if !in_hot_range {
                if let Some(archive_path) = archive_dir.as_ref() {
                    debug!(
                        "target_seq {} not in hot WAL, falling back to archive",
                        target_seq
                    );
                    let archive = archive_path.join(stream_id.to_string());
                    let mut archive_files =
                        list_wal_files(stream_id, &archive)?;
                    archive_files.sort_by_key(|f| f.first_seq);
                    if !archive_files.is_empty() {
                        (archive_files, archive, true)
                    } else {
                        (files, hot_dir.clone(), false)
                    }
                } else {
                    (files, hot_dir.clone(), false)
                }
            } else {
                (files, hot_dir.clone(), false)
            };

        // start from first file if target_seq is 0
        let file_idx = if files.is_empty() || target_seq == 0 {
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

        // if archive was empty, immediately transition to hot
        let (file, files, file_idx, wal_dir, in_archive) =
            if in_archive && file.is_none() {
                debug!(
                    "archive is empty, transitioning to hot WAL"
                );
                let mut hot_files =
                    list_wal_files(stream_id, &hot_dir)?;
                hot_files.sort_by_key(|f| f.first_seq);
                let f = if !hot_files.is_empty() {
                    Some(File::open(&hot_files[0].path)?)
                } else {
                    None
                };
                (f, hot_files, 0, hot_dir.clone(), false)
            } else {
                (file, files, file_idx, current_dir, in_archive)
            };

        Ok(Self {
            stream_id,
            wal_dir,
            hot_wal_dir: hot_dir,
            archive_dir: archive_dir.map(|p| p.to_path_buf()),
            file,
            files,
            file_idx,
            header_buf: [0u8; WalHeader::SIZE],
            in_archive,
        })
    }

    /// Read next record. Returns None at EOF.
    #[allow(clippy::should_implement_trait)]
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

            // MAX_PAYLOAD check removed - header.len is u16, already bounded

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

        // if we exhausted archive files, transition to hot WAL
        if self.in_archive && self.archive_dir.is_some() {
            debug!(
                "archive exhausted, transitioning to hot WAL"
            );
            self.in_archive = false;
            self.wal_dir = self.hot_wal_dir.clone();
            let mut hot_files = list_wal_files(
                self.stream_id,
                &self.hot_wal_dir,
            )?;
            hot_files.sort_by_key(|f| f.first_seq);
            self.files = hot_files;
            self.file_idx = 0;
            if !self.files.is_empty() {
                self.file = Some(File::open(
                    &self.files[0].path,
                )?);
                return Ok(true);
            }
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

/// Extract seq from first 8 bytes of payload
/// (CmpRecord convention: seq at offset 0).
pub fn extract_seq(payload: &[u8]) -> Option<u64> {
    if payload.len() < 8 {
        return None;
    }
    Some(u64::from_le_bytes([
        payload[0], payload[1], payload[2], payload[3],
        payload[4], payload[5], payload[6], payload[7],
    ]))
}
