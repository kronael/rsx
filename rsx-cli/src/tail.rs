use crate::inspect;
use crate::output::Failure;
use crate::segment::Frame;
use crate::segment::SegmentReader;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::wal::WalFileInfo;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
#[path = "tail_test.rs"]
mod tail_test;

struct OpenSegment {
    reader: SegmentReader,
    path: PathBuf,
    inode: u64,
    is_partial: bool,
    is_dated: bool,
}

/// Reads a live stream by byte offset: the segment being written stays open
/// and each poll reads only what was appended since. When the writer rotates
/// to a new segment the old one is drained and the next one opened.
pub struct Tail {
    stream_id: u32,
    wal_dir: PathBuf,
    next_seq: u64,
    current: Option<OpenSegment>,
    left_dated: Option<PathBuf>,
}

fn read_error(message: String) -> Failure {
    Failure::runtime(format!("wal read error: {}", message))
}

// a dated file has no sequence range, like the active one: told apart by name
fn is_dated(file: &WalFileInfo) -> bool {
    file.last_seq == u64::MAX && !file.path.to_string_lossy().ends_with("_active.wal")
}

impl Tail {
    pub fn new(stream_id: u32, wal_dir: &Path, from_seq: u64) -> Tail {
        Tail {
            stream_id,
            wal_dir: wal_dir.to_path_buf(),
            next_seq: from_seq,
            current: None,
            left_dated: None,
        }
    }

    /// The next record at or after the next sequence number, or `None` when
    /// the stream has nothing more yet. A corrupt record, or a partial one in
    /// a segment the writer has left, is a `Runtime` failure; a partial or bad
    /// record at the end of the segment still being written is waited out.
    pub fn next_record(&mut self) -> Result<Option<RawWalRecord>, Failure> {
        loop {
            if self.current.is_none() && !self.open_next()? {
                return Ok(None);
            }
            if let Some(raw) = self.read_current()? {
                return Ok(Some(raw));
            }
            if !self.has_later_segment()? {
                return Ok(None);
            }
            if let Some(raw) = self.read_current()? {
                return Ok(Some(raw));
            }
            self.leave_current()?;
        }
    }

    fn leave_current(&mut self) -> Result<(), Failure> {
        if let Some(open) = self.current.take() {
            if open.is_dated {
                self.left_dated = Some(open.path.clone());
            }
            if open.is_partial {
                return Err(read_error(format!(
                    "truncated record in {} at byte {}",
                    open.path.display(),
                    open.reader.offset
                )));
            }
        }
        Ok(())
    }

    fn list_segments(&self) -> Result<Vec<WalFileInfo>, Failure> {
        inspect::list_segments(self.stream_id, &self.wal_dir)
    }

    fn has_data_from(&self, file: &WalFileInfo) -> bool {
        let is_left = self
            .left_dated
            .as_ref()
            .is_some_and(|left| file.path <= *left);
        if is_dated(file) && is_left {
            return false;
        }
        file.is_active || file.last_seq >= self.next_seq
    }

    fn open_next(&mut self) -> Result<bool, Failure> {
        for file in self.list_segments()? {
            if !self.has_data_from(&file) {
                continue;
            }
            let reader = match SegmentReader::open(&file.path) {
                Ok(reader) => reader,
                Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
                Err(err) => {
                    return Err(read_error(format!(
                        "failed to open {}: {}",
                        file.path.display(),
                        err
                    )))
                }
            };
            let inode = reader
                .inode()
                .map_err(|err| read_error(format!("{}: {}", file.path.display(), err)))?;
            self.current = Some(OpenSegment {
                reader,
                inode,
                is_partial: false,
                is_dated: is_dated(&file),
                path: file.path,
            });
            return Ok(true);
        }
        Ok(false)
    }

    fn has_later_segment(&self) -> Result<bool, Failure> {
        let Some(open) = &self.current else {
            return Ok(false);
        };
        for file in self.list_segments()? {
            if !self.has_data_from(&file) {
                continue;
            }
            match std::fs::metadata(&file.path) {
                Ok(metadata) if std::os::unix::fs::MetadataExt::ino(&metadata) != open.inode => {
                    return Ok(true)
                }
                _ => {}
            }
        }
        Ok(false)
    }

    fn read_step(&mut self) -> Result<Step, Failure> {
        let Some(open) = self.current.as_mut() else {
            return Ok(Step::Idle);
        };
        let start = open.reader.offset;
        let frame = open
            .reader
            .read_frame()
            .map_err(|err| read_error(format!("{}: {}", open.path.display(), err)))?;
        match frame {
            None => {
                open.is_partial = false;
                Ok(Step::Idle)
            }
            Some(Frame::Truncated) => {
                open.reader
                    .seek_to(start)
                    .map_err(|err| read_error(format!("{}: {}", open.path.display(), err)))?;
                open.is_partial = true;
                Ok(Step::Idle)
            }
            Some(Frame::BadHeader) => Err(read_error(format!(
                "unknown header version in {} at byte {}",
                open.path.display(),
                start
            ))),
            Some(Frame::Record {
                record_type,
                payload,
                is_crc_ok: true,
            }) => Ok(Step::Record(record_type, payload)),
            Some(Frame::Record {
                is_crc_ok: false, ..
            }) => {
                let has_more = open
                    .reader
                    .has_bytes_after()
                    .map_err(|err| read_error(format!("{}: {}", open.path.display(), err)))?;
                Ok(Step::BadCrc { start, has_more })
            }
        }
    }

    fn read_current(&mut self) -> Result<Option<RawWalRecord>, Failure> {
        loop {
            match self.read_step()? {
                Step::Idle => return Ok(None),
                Step::BadCrc { start, has_more } => {
                    if has_more || self.has_later_segment()? {
                        return Err(read_error(format!(
                            "crc mismatch in {} at byte {}",
                            self.path(),
                            start
                        )));
                    }
                    self.wait_at(start)?;
                    return Ok(None);
                }
                Step::Record(record_type, payload) => {
                    let seq = extract_seq(&payload).unwrap_or(0);
                    if seq < self.next_seq {
                        continue;
                    }
                    self.next_seq = seq.saturating_add(1);
                    let crc32 = compute_crc32(&payload);
                    return Ok(Some(RawWalRecord {
                        header: WalHeader::new(record_type, payload.len() as u16, crc32),
                        payload,
                    }));
                }
            }
        }
    }

    fn path(&self) -> String {
        self.current
            .as_ref()
            .map_or(String::new(), |open| open.path.display().to_string())
    }

    fn wait_at(&mut self, offset: u64) -> Result<(), Failure> {
        let path = self.path();
        if let Some(open) = self.current.as_mut() {
            open.reader
                .seek_to(offset)
                .map_err(|err| read_error(format!("{}: {}", path, err)))?;
            open.is_partial = false;
        }
        Ok(())
    }
}

enum Step {
    Record(u16, Vec<u8>),
    Idle,
    BadCrc { start: u64, has_more: bool },
}
