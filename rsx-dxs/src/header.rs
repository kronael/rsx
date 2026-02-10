/// WAL record header (16 bytes exactly)
///
/// Layout: record_type(2) + len(2) + crc32(4)
/// + _reserved(8) = 16 bytes.
/// Manual encode/decode, no repr(packed).
#[derive(Debug, Clone, Copy)]
pub struct WalHeader {
    pub record_type: u16,
    pub len: u16,
    pub crc32: u32,
    pub _reserved: [u8; 8],
}

impl WalHeader {
    pub const SIZE: usize = 16;

    pub fn new(
        record_type: u16,
        len: u16,
        crc32: u32,
    ) -> Self {
        Self {
            record_type,
            len,
            crc32,
            _reserved: [0u8; 8],
        }
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..2].copy_from_slice(
            &self.record_type.to_le_bytes(),
        );
        buf[2..4].copy_from_slice(
            &self.len.to_le_bytes(),
        );
        buf[4..8].copy_from_slice(
            &self.crc32.to_le_bytes(),
        );
        // bytes 8..16 stay zeroed (_reserved)
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            record_type: u16::from_le_bytes(
                [buf[0], buf[1]],
            ),
            len: u16::from_le_bytes(
                [buf[2], buf[3]],
            ),
            crc32: u32::from_le_bytes([
                buf[4], buf[5], buf[6], buf[7],
            ]),
            _reserved: [
                buf[8], buf[9], buf[10], buf[11],
                buf[12], buf[13], buf[14], buf[15],
            ],
        })
    }
}
