use crate::output::show;
use crate::output::Failure;
use crate::segment::Frame;
use crate::segment::SegmentReader;
use crate::wal::extract_ts_ns;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::list_wal_files_across;
use rsx_cast::wal::WalFileInfo;
use serde_json::json;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

#[cfg(test)]
#[path = "inspect_test.rs"]
mod inspect_test;

/// One segment file of a stream.
pub struct SegmentListing {
    pub name: String,
    pub bytes: u64,
    pub modified_ms: u64,
    pub is_active: bool,
}

/// What `wal ls` reports for one stream directory.
pub struct StreamListing {
    pub stream_id: u32,
    pub files: usize,
    pub bytes: u64,
    pub segments: Vec<SegmentListing>,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub last_ts_ns: Option<u64>,
}

/// What `wal verify` reports for one stream directory.
pub struct StreamVerdict {
    pub stream_id: u32,
    pub records: u64,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub crc_errors: u64,
    pub seq_gaps: Vec<(u64, u64)>,
    pub torn_tail: bool,
    pub error: Option<String>,
}

impl StreamListing {
    pub fn to_json(&self) -> Value {
        let segments: Vec<Value> = self
            .segments
            .iter()
            .map(|segment| {
                json!({
                    "name": segment.name,
                    "bytes": segment.bytes,
                    "modified_ms": segment.modified_ms,
                    "is_active": segment.is_active,
                })
            })
            .collect();
        json!({
            "stream_id": self.stream_id,
            "files": self.files,
            "bytes": self.bytes,
            "segments": segments,
            "first_seq": self.first_seq,
            "last_seq": self.last_seq,
            "last_ts_ns": self.last_ts_ns,
        })
    }
}

impl StreamVerdict {
    /// A torn tail alone is not corruption: it is what a crash or a read of
    /// a live segment mid-write leaves.
    pub fn is_corrupt(&self) -> bool {
        self.crc_errors > 0 || !self.seq_gaps.is_empty() || self.error.is_some()
    }

    pub fn to_text(&self) -> String {
        let state = if self.is_corrupt() { "corrupt" } else { "ok" };
        let mut line = format!(
            "stream {}: {} records={} seq={}..{} crc_errors={} torn_tail={}",
            self.stream_id,
            state,
            self.records,
            show(self.first_seq),
            show(self.last_seq),
            self.crc_errors,
            self.torn_tail,
        );
        for (from, to) in &self.seq_gaps {
            line.push_str(&format!(" seq_gap={}..{}", from, to));
        }
        if let Some(error) = &self.error {
            line.push_str(&format!(" error: {}", error));
        }
        line
    }

    pub fn to_json(&self) -> Value {
        json!({
            "stream_id": self.stream_id,
            "records": self.records,
            "first_seq": self.first_seq,
            "last_seq": self.last_seq,
            "crc_errors": self.crc_errors,
            "seq_gaps": self.seq_gaps,
            "torn_tail": self.torn_tail,
            "error": self.error,
        })
    }
}

/// The streams to inspect: the numeric subdirectories of `wal_dir`, or
/// exactly `only` when it is not empty. Fails with `Runtime` when `wal_dir`
/// cannot be read or a requested stream has no directory.
pub fn find_streams(wal_dir: &Path, only: &[u32]) -> Result<Vec<u32>, Failure> {
    let entries = fs::read_dir(wal_dir).map_err(|err| {
        Failure::runtime(format!("failed to read {}: {}", wal_dir.display(), err))
    })?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            Failure::runtime(format!("failed to read {}: {}", wal_dir.display(), err))
        })?;
        let Ok(stream_id) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if entry.path().is_dir() {
            found.push(stream_id);
        }
    }
    found.sort_unstable();
    if only.is_empty() {
        return Ok(found);
    }
    let mut wanted = only.to_vec();
    wanted.sort_unstable();
    wanted.dedup();
    for stream_id in &wanted {
        if !found.contains(stream_id) {
            return Err(Failure::runtime(format!(
                "no wal directory for stream {} under {}",
                stream_id,
                wal_dir.display()
            )));
        }
    }
    Ok(wanted)
}

pub(crate) fn list_segments(stream_id: u32, wal_dir: &Path) -> Result<Vec<WalFileInfo>, Failure> {
    let stream_dir = wal_dir.join(stream_id.to_string());
    list_wal_files_across(stream_id, &[stream_dir.as_path()]).map_err(|err| {
        Failure::runtime(format!("failed to list {}: {}", stream_dir.display(), err))
    })
}

fn open_segment(file: &WalFileInfo) -> Result<SegmentReader, Failure> {
    SegmentReader::open(&file.path)
        .map_err(|err| Failure::runtime(format!("failed to open {}: {}", file.path.display(), err)))
}

fn read_error(file: &WalFileInfo, err: std::io::Error) -> Failure {
    Failure::runtime(format!("failed to read {}: {}", file.path.display(), err))
}

enum End {
    First,
    Last,
}

fn read_intact_record(file: &WalFileInfo, end: End) -> Result<Option<(u64, u64)>, Failure> {
    let mut reader = open_segment(file)?;
    let mut found = None;
    loop {
        let frame = reader.read_frame().map_err(|err| read_error(file, err))?;
        match frame {
            None | Some(Frame::Truncated) | Some(Frame::BadHeader) => return Ok(found),
            Some(Frame::Record {
                is_crc_ok: false, ..
            }) => {}
            Some(Frame::Record { payload, .. }) => {
                let (Some(seq), Some(ts_ns)) = (extract_seq(&payload), extract_ts_ns(&payload))
                else {
                    continue;
                };
                found = Some((seq, ts_ns));
                if matches!(end, End::First) {
                    return Ok(found);
                }
            }
        }
    }
}

/// Lists one stream: segment count, byte total, and the first and last intact
/// record. Reads only the segments needed to find those two records.
pub fn list_stream(stream_id: u32, wal_dir: &Path) -> Result<StreamListing, Failure> {
    let segments = list_segments(stream_id, wal_dir)?;
    let mut bytes = 0;
    let mut listed = Vec::with_capacity(segments.len());
    for file in &segments {
        let metadata = fs::metadata(&file.path).map_err(|err| read_error(file, err))?;
        bytes += metadata.len();
        let modified = metadata.modified().map_err(|err| read_error(file, err))?;
        let modified_ms = modified
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        listed.push(SegmentListing {
            name: file
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            bytes: metadata.len(),
            modified_ms,
            is_active: file.is_active,
        });
    }
    let mut first = None;
    for file in &segments {
        first = read_intact_record(file, End::First)?;
        if first.is_some() {
            break;
        }
    }
    let mut last = None;
    for file in segments.iter().rev() {
        last = read_intact_record(file, End::Last)?;
        if last.is_some() {
            break;
        }
    }
    Ok(StreamListing {
        stream_id,
        files: segments.len(),
        bytes,
        segments: listed,
        first_seq: first.map(|(seq, _)| seq),
        last_seq: last.map(|(seq, _)| seq),
        last_ts_ns: last.map(|(_, ts_ns)| ts_ns),
    })
}

/// Checks the CRC of every record of every segment of a stream, the sequence
/// continuity across segments, and the tail of the last segment. I/O errors
/// fail the command; findings go into the verdict.
pub fn verify_stream(stream_id: u32, wal_dir: &Path) -> Result<StreamVerdict, Failure> {
    let segments = list_segments(stream_id, wal_dir)?;
    let mut verdict = StreamVerdict {
        stream_id,
        records: 0,
        first_seq: None,
        last_seq: None,
        crc_errors: 0,
        seq_gaps: Vec::new(),
        torn_tail: false,
        error: None,
    };
    let mut previous: Option<u64> = None;
    for (index, file) in segments.iter().enumerate() {
        let is_last_segment = index + 1 == segments.len();
        let mut reader = open_segment(file)?;
        loop {
            let offset = reader.offset;
            let frame = reader.read_frame().map_err(|err| read_error(file, err))?;
            match frame {
                None => break,
                Some(Frame::Record {
                    is_crc_ok: false, ..
                }) => {
                    verdict.crc_errors += 1;
                    previous = previous.map(|seq| seq.saturating_add(1));
                }
                Some(Frame::Record { payload, .. }) => {
                    verdict.records += 1;
                    let Some(seq) = extract_seq(&payload) else {
                        continue;
                    };
                    match previous {
                        Some(last) if seq > last.saturating_add(1) => {
                            verdict.seq_gaps.push((last + 1, seq - 1));
                        }
                        Some(last) if seq <= last => {
                            verdict.error.get_or_insert(format!(
                                "{}: seq {} follows {} at byte {}",
                                file.path.display(),
                                seq,
                                last,
                                offset
                            ));
                        }
                        _ => {}
                    }
                    previous = Some(seq);
                    verdict.first_seq = verdict.first_seq.or(Some(seq));
                    verdict.last_seq = Some(seq);
                }
                Some(Frame::Truncated) if is_last_segment => {
                    verdict.torn_tail = true;
                    break;
                }
                Some(Frame::Truncated) => {
                    verdict.error.get_or_insert(format!(
                        "{}: truncated record at byte {}",
                        file.path.display(),
                        offset
                    ));
                    break;
                }
                Some(Frame::BadHeader) => {
                    verdict.error.get_or_insert(format!(
                        "{}: unknown header version at byte {}",
                        file.path.display(),
                        offset
                    ));
                    break;
                }
            }
        }
    }
    Ok(verdict)
}
