//! `WalWriter` + `WalReader` + `read_record_at_seq`. See `specs/48-wal.md`.

use crate::encode_utils::as_bytes;
use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::CastRecord;
use std::fs;
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

/// Capacity of the inline wire buffer in `Framed`. Sized to
/// hold the 16-byte WAL header plus the largest current record
/// payload (128 B: FillRecord, BboRecord, OrderAcceptedRecord)
/// with headroom. A compile-time assert in `prepare` guards
/// this against future record growth.
pub const FRAMED_WIRE_BYTES: usize = 256;

/// A record framed exactly once for both persistence and
/// publication. Returned by `WalWriter::prepare`.
///
/// `wire[..total]` contains `[header (16 B) | payload]` ready
/// to pass directly to `send_to` or `extend_from_slice`.
pub struct Framed {
    pub wire: [u8; FRAMED_WIRE_BYTES],
    pub total: u16,
    pub seq: u64,
}

/// Records buffered between flushes before `should_flush` returns
/// `true`. Producers call `should_flush` after each `append_framed`
/// to decide when to drain the buffer to disk. Sized to amortise
/// fsync cost across a meaningful batch without growing the
/// in-memory buffer past ~64 KB at typical record sizes.
const FLUSH_RECORD_THRESHOLD: u32 = 1000;

// ── Path layout ──────────────────────────────────────────────
//
// File names encode (stream_id, first_seq, last_seq). The
// active (still-being-written) file uses a separate name with
// `_active` instead of seqs so the rotation code never has to
// rename while writers can be in flight. These helpers are the
// ONLY place path strings are constructed; everything else
// goes through them.

/// `wal_dir/<stream_id>/` — one segment directory per stream.
fn stream_dir(wal_dir: &Path, stream_id: u32) -> PathBuf {
    wal_dir.join(stream_id.to_string())
}

/// Bare filename of the active (still-being-written) segment.
fn active_filename(stream_id: u32) -> String {
    format!("{}_active.wal", stream_id)
}

/// Bare filename of a rotated, frozen segment covering
/// seqs `first..=last`.
fn segment_filename(
    stream_id: u32,
    first_seq: u64,
    last_seq: u64,
) -> String {
    format!("{}_{}_{}.wal", stream_id, first_seq, last_seq)
}

/// Full path: `<wal_dir>/<stream_id>/<stream_id>_active.wal`.
fn active_file_path(wal_dir: &Path, stream_id: u32) -> PathBuf {
    stream_dir(wal_dir, stream_id).join(active_filename(stream_id))
}

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
        let dir = stream_dir(wal_dir, stream_id);
        fs::create_dir_all(&dir)?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(active_file_path(wal_dir, stream_id))?;

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

    /// Assign seq, compute CRC, pack `[header | payload]` into
    /// the returned `Framed`. The single point at which a record
    /// is encoded for both WAL persistence and live UDP send.
    ///
    /// Paired callers (those that both persist AND publish the
    /// same record) MUST use this entry point — calling
    /// `append` and `CastSender::send` separately on the same
    /// record recomputes CRC and the seq counters can drift.
    /// See `notes/crc.md`.
    pub fn prepare<T: CastRecord>(
        &mut self,
        record: &mut T,
    ) -> io::Result<Framed> {
        let payload_len = std::mem::size_of::<T>();
        let total = WalHeader::SIZE + payload_len;
        assert!(
            total <= FRAMED_WIRE_BYTES,
            "record too large for Framed wire buffer: {} > {}",
            total,
            FRAMED_WIRE_BYTES,
        );

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

        let mut wire = [0u8; FRAMED_WIRE_BYTES];
        wire[..WalHeader::SIZE].copy_from_slice(header.to_bytes());
        wire[WalHeader::SIZE..total].copy_from_slice(payload);

        Ok(Framed { wire, total: total as u16, seq })
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
        self.buf.extend_from_slice(&framed.wire[..framed.total as usize]);
        self.records_since_flush += 1;
        Ok(())
    }


    /// Discard the in-memory write buffer without flushing.
    /// Used in benchmarks to prevent unbounded allocation across
    /// Criterion warmup iterations.
    pub fn reset_write_buf(&mut self) {
        self.buf.clear();
        self.records_since_flush = 0;
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
        let active_path = self.wal_dir
            .join(active_filename(self.stream_id));
        let rotated_path = self.wal_dir.join(segment_filename(
            self.stream_id,
            self.first_seq,
            self.last_seq,
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
        self.records_since_flush >= FLUSH_RECORD_THRESHOLD
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

/// Merge WAL files across one or more directories into a
/// single list sorted by `first_seq`. The active file
/// (sentinel `first_seq=u64::MAX`) sorts last. Missing
/// directories are treated as empty.
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
/// for `stream_id`. `None` if no records are present.
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
/// Returns Ok(Some(_)) if found, Ok(None) if not present
/// in any visible WAL file, Err(_) on IO failures.
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

#[cfg(test)]
#[path = "wal_test.rs"]
mod wal_test;
