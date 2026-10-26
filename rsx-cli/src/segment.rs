use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::Read;
use std::path::Path;

#[cfg(test)]
#[path = "segment_test.rs"]
mod segment_test;

/// One frame of a segment file.
pub enum Frame {
    /// A complete record; `is_crc_ok` is false when the payload does not match
    /// the CRC in its header.
    Record {
        record_type: u16,
        payload: Vec<u8>,
        is_crc_ok: bool,
    },
    /// The file ends inside a header or a payload.
    Truncated,
    /// The header carries an unknown version byte, so its length is unusable.
    BadHeader,
}

/// Sequential reader over one segment file that reports every frame,
/// including the ones with a bad CRC. `WalReader` stops at the first bad
/// record; this reader keeps going, because the header length still locates
/// the next record.
pub struct SegmentReader {
    file: BufReader<File>,
    /// Byte offset of the next frame.
    pub offset: u64,
}

impl SegmentReader {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(SegmentReader {
            file: BufReader::new(File::open(path)?),
            offset: 0,
        })
    }

    /// Returns `None` at a clean end of file. After `Truncated` or
    /// `BadHeader` the reader must not be used again.
    pub fn read_frame(&mut self) -> io::Result<Option<Frame>> {
        let mut header_buf = [0u8; WalHeader::SIZE];
        let read = read_up_to(&mut self.file, &mut header_buf)?;
        if read == 0 {
            return Ok(None);
        }
        if read < WalHeader::SIZE {
            return Ok(Some(Frame::Truncated));
        }
        let Some(header) = WalHeader::from_bytes(&header_buf) else {
            return Ok(Some(Frame::BadHeader));
        };
        let mut payload = vec![0u8; header.len as usize];
        if read_up_to(&mut self.file, &mut payload)? < payload.len() {
            return Ok(Some(Frame::Truncated));
        }
        self.offset += (WalHeader::SIZE + payload.len()) as u64;
        let is_crc_ok = compute_crc32(&payload) == header.crc32;
        Ok(Some(Frame::Record {
            record_type: header.record_type,
            payload,
            is_crc_ok,
        }))
    }
}

fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let read = reader.read(&mut buf[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}
