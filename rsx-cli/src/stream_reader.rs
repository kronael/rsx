use crate::inspect;
use crate::output::Failure;
use crate::segment::Frame;
use crate::segment::SegmentReader;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::wal::WalFileInfo;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::vec::IntoIter;

#[cfg(test)]
#[path = "stream_reader_test.rs"]
mod stream_reader_test;

/// Reads a stream's records in file order across every segment
/// `inspect::list_segments` names: the WAL writer's files, then the
/// recorder's dated archive files. Like `WalReader`, a partial record or a
/// bad CRC with nothing after it in the last segment is the torn tail of a
/// crash and ends the stream; a bad record anywhere else is a `Runtime`
/// failure.
pub struct StreamReader {
    pending: IntoIter<WalFileInfo>,
    current: Option<(PathBuf, SegmentReader)>,
}

fn read_error(path: &Path, message: impl Display) -> Failure {
    Failure::runtime(format!("wal read error: {}: {}", path.display(), message))
}

impl StreamReader {
    /// Opens the stream at the first segment that can hold `from_seq`; the
    /// caller skips the records before it inside that segment.
    pub fn open(stream_id: u32, from_seq: u64, wal_dir: &Path) -> Result<Self, Failure> {
        let mut segments = inspect::list_segments(stream_id, wal_dir)?;
        segments.retain(|file| file.last_seq >= from_seq);
        Ok(StreamReader {
            pending: segments.into_iter(),
            current: None,
        })
    }

    pub fn next_record(&mut self) -> Result<Option<RawWalRecord>, Failure> {
        loop {
            let Some((path, reader)) = self.current.as_mut() else {
                let Some(file) = self.pending.next() else {
                    return Ok(None);
                };
                let reader =
                    SegmentReader::open(&file.path).map_err(|err| read_error(&file.path, err))?;
                self.current = Some((file.path, reader));
                continue;
            };
            let start = reader.offset;
            let frame = reader.read_frame().map_err(|err| read_error(path, err))?;
            let is_last_segment = self.pending.len() == 0;
            match frame {
                None => self.current = None,
                Some(Frame::Record {
                    record_type,
                    payload,
                    is_crc_ok: true,
                }) => {
                    let crc32 = compute_crc32(&payload);
                    return Ok(Some(RawWalRecord {
                        header: WalHeader::new(record_type, payload.len() as u16, crc32),
                        payload,
                    }));
                }
                Some(Frame::Record {
                    is_crc_ok: false, ..
                }) => {
                    let has_more = reader
                        .has_bytes_after()
                        .map_err(|err| read_error(path, err))?;
                    if is_last_segment && !has_more {
                        return Ok(None);
                    }
                    return Err(read_error(path, format!("crc mismatch at byte {start}")));
                }
                Some(Frame::Truncated) if is_last_segment => return Ok(None),
                Some(Frame::Truncated) => {
                    return Err(read_error(
                        path,
                        format!("truncated record at byte {start}"),
                    ));
                }
                Some(Frame::BadHeader) => {
                    return Err(read_error(
                        path,
                        format!("unknown header version at byte {start}"),
                    ));
                }
            }
        }
    }
}
