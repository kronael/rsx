//! WAL — the substrate shared by casting and replication.
//!
//! Append-only sequence of `WalHeader` + payload records,
//! rotated into fixed-size segment files (default 64 MiB).
//! The bytes on disk are identical to the bytes sent over UDP
//! by `CastSender` and over TCP by `ReplicationService` —
//! there is no serialization step.
//!
//! `WalWriter` owns the active segment; `WalReader` iterates
//! either forward (replay) or random-access by seq
//! (`read_record_at_seq`, used for cold-tier NAK retransmits).
//! `oldest_and_highest_seq` and `list_wal_files_across` let
//! the replication server / archive consumers reason about
//! coverage across multiple WAL directories.

use crate::encode_utils::as_bytes;
use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::CastRecord;
use std::fs;

/// A record framed exactly once for both persistence and
/// publication. Returned by `WalWriter::prepare`. The borrow
/// of `payload` keeps the source record locked against
/// mutation until `Framed` is dropped, so the bytes the WAL
/// writes and the bytes the sender publishes are byte-equal
/// by construction.
pub struct Framed<'a> {
    pub header: WalHeader,
    pub payload: &'a [u8],
    pub seq: u64,
}
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
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
    max_file_size: u64,
    records_since_flush: u32,
}

impl WalWriter {
    pub fn new(
        stream_id: u32,
        wal_dir: &Path,
        max_file_size: u64,
    ) -> io::Result<Self> {
        let dir = wal_dir.join(stream_id.to_string());
        fs::create_dir_all(&dir)?;

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
            max_file_size,
            records_since_flush: 0,
        })
    }

    /// Set the next seq to assign. Used post-replay so live
    /// writes don't re-use a seq already on disk. R-N1.
    pub fn set_next_seq(&mut self, seq: u64) {
        self.next_seq = seq;
        if self.last_seq < seq.saturating_sub(1) {
            self.last_seq = seq.saturating_sub(1);
        }
    }

    /// Assign seq, compute CRC, build header. The single point
    /// at which a record is "framed" for both WAL persistence
    /// and live UDP send.
    ///
    /// Paired callers (those that both persist AND publish the
    /// same record) MUST use this entry point — calling
    /// `append` and `CastSender::send` separately on the same
    /// record recomputes CRC and the seq counters can drift.
    /// See `notes/crc.md`.
    pub fn prepare<'a, T: CastRecord>(
        &mut self,
        record: &'a mut T,
    ) -> io::Result<Framed<'a>> {
        let payload_len = std::mem::size_of::<T>();
        if payload_len > MAX_PAYLOAD as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload exceeds 64KB",
            ));
        }

        let seq = self.next_seq;
        self.next_seq += 1;
        record.set_seq(seq);

        let payload = as_bytes(record);
        let crc = compute_crc32(payload);
        let header = WalHeader::new(
            T::record_type(),
            payload_len as u16,
            crc,
        );

        Ok(Framed { header, payload, seq })
    }

    /// Append a pre-framed record to the in-memory buffer.
    /// No CRC compute, no seq assignment — caller already paid
    /// those costs in `prepare`. O(memcpy).
    pub fn append_framed(
        &mut self,
        framed: &Framed,
    ) -> io::Result<()> {
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
        self.last_seq = framed.seq;
        self.buf.extend_from_slice(&framed.header.to_bytes());
        self.buf.extend_from_slice(framed.payload);
        self.records_since_flush += 1;
        Ok(())
    }

    /// Convenience for solo-WAL callers (write to disk without
    /// also publishing over UDP). Wraps `prepare` +
    /// `append_framed` so the CRC is still computed exactly
    /// once.
    pub fn append<T: CastRecord>(
        &mut self,
        record: &mut T,
    ) -> io::Result<u64> {
        let framed = self.prepare(record)?;
        let seq = framed.seq;
        self.append_framed(&framed)?;
        Ok(seq)
    }

    /// Flush buffer to disk with fsync.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }

        // rotate before writing if file has data and adding
        // the buffer would exceed the size limit
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
        }

        // rotate after write if file has now reached the limit
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

        Ok(())
    }

    pub fn should_flush(&self) -> bool {
        self.records_since_flush >= 1000
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
        let hot_dir = wal_dir.join(stream_id.to_string());
        let dirs = [hot_dir.as_path()];
        let files = list_wal_files_across(stream_id, &dirs)?;
        let file_idx = pick_start_idx(&files, target_seq);

        let file = if file_idx < files.len() {
            Some(File::open(&files[file_idx].path)?)
        } else {
            None
        };

        Ok(Self {
            stream_id,
            wal_dir: hot_dir,
            file,
            files,
            file_idx,
            header_buf: [0u8; WalHeader::SIZE],
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

/// Merge WAL files across multiple directories (e.g.
/// hot and archive) into one list sorted by `first_seq`.
/// The active file (sentinel `first_seq=u64::MAX`) sorts
/// last. Missing directories are treated as empty.
pub fn list_wal_files_across(
    stream_id: u32,
    dirs: &[&Path],
) -> io::Result<Vec<WalFileInfo>> {
    let mut files = Vec::new();
    for dir in dirs {
        for f in list_wal_files(stream_id, dir)? {
            files.push(f);
        }
    }
    files.sort_by_key(|f| f.first_seq);
    Ok(files)
}

/// Pick the index in `files` (sorted by `first_seq`) where
/// iteration should start for `target_seq`. Active file
/// uses `u64::MAX` sentinels and is treated as the last
/// entry.
fn pick_start_idx(
    files: &[WalFileInfo],
    target_seq: u64,
) -> usize {
    if files.is_empty() || target_seq == 0 {
        return 0;
    }
    let mut idx = 0;
    for (i, f) in files.iter().enumerate() {
        if !f.is_active
            && target_seq >= f.first_seq
            && target_seq <= f.last_seq
        {
            return i;
        }
        idx = i;
    }
    idx
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
/// (CastRecord convention: seq at offset 0).
pub fn extract_seq(payload: &[u8]) -> Option<u64> {
    if payload.len() < 8 {
        return None;
    }
    Some(u64::from_le_bytes([
        payload[0], payload[1], payload[2], payload[3],
        payload[4], payload[5], payload[6], payload[7],
    ]))
}

/// Return the (oldest, highest) seq range currently on disk
/// for `stream_id` across the given directories (typically
/// hot + optional archive). `None` if no records are present.
///
/// Used by the replay server to pre-check whether the
/// requested `from_seq` is in-range before opening a reader.
/// If the request is below `oldest`, the server emits a
/// `ReplicationNotAvailable` and closes the connection so the
/// consumer can try the next endpoint.
///
/// The active file's first/last seq aren't encoded in its
/// filename (sentinel `u64::MAX`); we discover its real
/// range by scanning it. Rotated files use the filename
/// directly. Returns `None` when the active file is empty
/// AND no rotated files exist.
pub fn oldest_and_highest_seq(
    stream_id: u32,
    wal_dir: &Path,
) -> io::Result<Option<(u64, u64)>> {
    let hot_dir = wal_dir.join(stream_id.to_string());
    let dirs = [hot_dir.as_path()];
    let files = list_wal_files_across(stream_id, &dirs)?;
    let mut oldest: Option<u64> = None;
    let mut highest: Option<u64> = None;

    for f in &files {
        if f.is_active {
            // Scan the active file end-to-end to discover its
            // seq range. Active files are bounded by
            // max_file_size (64 MB default).
            if let Some((lo, hi)) = scan_file_seq_range(&f.path)? {
                oldest = Some(oldest.map_or(lo, |o| o.min(lo)));
                highest = Some(highest.map_or(hi, |h| h.max(hi)));
            }
        } else {
            oldest = Some(oldest.map_or(f.first_seq, |o| o.min(f.first_seq)));
            highest = Some(highest.map_or(f.last_seq, |h| h.max(f.last_seq)));
        }
    }

    match (oldest, highest) {
        (Some(o), Some(h)) => Ok(Some((o, h))),
        _ => Ok(None),
    }
}

fn scan_file_seq_range(
    path: &Path,
) -> io::Result<Option<(u64, u64)>> {
    let mut file = File::open(path)?;
    let mut hdr_buf = [0u8; WalHeader::SIZE];
    let mut lo: Option<u64> = None;
    let mut hi: Option<u64> = None;
    loop {
        match file.read_exact(&mut hdr_buf) {
            Ok(()) => {}
            Err(e)
                if e.kind()
                    == io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(e),
        }
        let header = match WalHeader::from_bytes(&hdr_buf) {
            Some(h) => h,
            None => break,
        };
        let mut payload = vec![0u8; header.len as usize];
        match file.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e)
                if e.kind()
                    == io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(e),
        }
        if compute_crc32(&payload) != header.crc32 {
            break;
        }
        if let Some(seq) = extract_seq(&payload) {
            lo = Some(lo.map_or(seq, |l| l.min(seq)));
            hi = Some(hi.map_or(seq, |h| h.max(seq)));
        }
    }
    match (lo, hi) {
        (Some(l), Some(h)) => Ok(Some((l, h))),
        _ => Ok(None),
    }
}

/// Random-access read of a single WAL record by seq.
///
/// Used by CMP NAK retransmit when the requested seq has
/// fallen out of the in-memory send_ring (default 4096).
/// The cost is one file open + a sequential scan within
/// that file (≤ 64 MB by default rotation), so a NAK
/// for a 1-hour-old record becomes "open one file + scan
/// it" rather than impossible.
///
/// Returns Ok(Some(_)) if found, Ok(None) if the seq
/// isn't present in any visible WAL file (already GC'd
/// past the retention window), Err(_) on IO failures.
pub fn read_record_at_seq(
    stream_id: u32,
    target_seq: u64,
    wal_dir: &Path,
) -> io::Result<Option<RawWalRecord>> {
    let hot_dir = wal_dir.join(stream_id.to_string());
    let dirs = [hot_dir.as_path()];
    let files = list_wal_files_across(stream_id, &dirs)?;
    if let Some(target) = pick_file_for_seq(&files, target_seq)
    {
        if let Some(rec) = scan_file_for_seq(
            &target.path, target_seq,
        )? {
            return Ok(Some(rec));
        }
    }
    Ok(None)
}

fn pick_file_for_seq(
    files: &[WalFileInfo],
    target_seq: u64,
) -> Option<&WalFileInfo> {
    // Rotated files have first_seq..=last_seq populated.
    // Active file has u64::MAX sentinels — always check
    // it last in case the seq is post-rotation.
    for f in files {
        if !f.is_active
            && target_seq >= f.first_seq
            && target_seq <= f.last_seq
        {
            return Some(f);
        }
    }
    files.iter().find(|f| f.is_active)
}

fn scan_file_for_seq(
    path: &Path,
    target_seq: u64,
) -> io::Result<Option<RawWalRecord>> {
    let mut file = File::open(path)?;
    let mut hdr_buf = [0u8; WalHeader::SIZE];
    loop {
        match file.read_exact(&mut hdr_buf) {
            Ok(()) => {}
            Err(e)
                if e.kind()
                    == io::ErrorKind::UnexpectedEof =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        let header = match WalHeader::from_bytes(&hdr_buf) {
            Some(h) => h,
            None => return Ok(None),
        };
        let mut payload = vec![0u8; header.len as usize];
        match file.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e)
                if e.kind()
                    == io::ErrorKind::UnexpectedEof =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        if compute_crc32(&payload) != header.crc32 {
            // Skip corrupt records rather than aborting —
            // we may still find the target later in the
            // same file.
            continue;
        }
        if let Some(seq) = extract_seq(&payload) {
            if seq == target_seq {
                return Ok(Some(RawWalRecord {
                    header,
                    payload,
                }));
            }
            if seq > target_seq {
                // Past the target without finding it.
                return Ok(None);
            }
        }
    }
}
