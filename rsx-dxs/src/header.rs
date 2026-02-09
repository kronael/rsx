/// WAL record header (16 bytes exactly)
///
/// Layout: version(2) + record_type(2) + len(4) + stream_id(4)
/// + crc32(4) = 16 bytes. Manual encode/decode, no repr(packed).
#[derive(Debug, Clone, Copy)]
pub struct WalHeader {
    pub version: u16,
    pub record_type: u16,
    pub len: u32,
    pub stream_id: u32,
    pub crc32: u32,
}

impl WalHeader {
    pub const SIZE: usize = 16;
    pub const VERSION: u16 = 1;

    pub fn new(
        record_type: u16,
        len: u32,
        stream_id: u32,
        crc32: u32,
    ) -> Self {
        Self {
            version: Self::VERSION,
            record_type,
            len,
            stream_id,
            crc32,
        }
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..2].copy_from_slice(
            &self.version.to_le_bytes(),
        );
        buf[2..4].copy_from_slice(
            &self.record_type.to_le_bytes(),
        );
        buf[4..8].copy_from_slice(
            &self.len.to_le_bytes(),
        );
        buf[8..12].copy_from_slice(
            &self.stream_id.to_le_bytes(),
        );
        buf[12..16].copy_from_slice(
            &self.crc32.to_le_bytes(),
        );
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            version: u16::from_le_bytes([buf[0], buf[1]]),
            record_type: u16::from_le_bytes([buf[2], buf[3]]),
            len: u32::from_le_bytes([
                buf[4], buf[5], buf[6], buf[7],
            ]),
            stream_id: u32::from_le_bytes([
                buf[8], buf[9], buf[10], buf[11],
            ]),
            crc32: u32::from_le_bytes([
                buf[12], buf[13], buf[14], buf[15],
            ]),
        })
    }
}
