/// WAL record header (16 bytes exactly).
///
/// Layout (LE on the wire):
/// ```text
/// offset 0      version:     u8    (WalVersion as u8)
/// offset 1      _pad:        u8
/// offset 2..4   record_type: u16
/// offset 4..6   len:         u16   (payload length, bytes)
/// offset 6..8   _pad:        u16
/// offset 8..12  crc32:       u32   (CRC32C of payload)
/// offset 12..16 _reserved:   [u8; 4]
/// ```
///
/// Version is first so receivers can gate on it before
/// interpreting any other field. `from_bytes` returns `None`
/// for unrecognised versions; callers never see an invalid
/// `WalHeader`.
///
/// Adding a new record type does NOT bump the version
/// (additive). Bump only for header-layout or CRC-algorithm
/// changes; those require a coordinated stop-redeploy.
#[derive(Debug, Clone, Copy)]
pub struct WalHeader {
    pub version: WalVersion,
    pub record_type: u16,
    pub len: u16,
    pub crc32: u32,
    /// Absorbs: pad@1, pad@6..8, reserved@12..16.
    pub _reserved: [u8; 7],
}

/// Known wire-format versions.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalVersion {
    V1 = 1,
}

impl TryFrom<u8> for WalVersion {
    type Error = ();
    fn try_from(v: u8) -> Result<Self, ()> {
        match v {
            1 => Ok(WalVersion::V1),
            _ => Err(()),
        }
    }
}

impl WalHeader {
    pub const SIZE: usize = 16;

    pub fn new(
        record_type: u16,
        len: u16,
        crc32: u32,
    ) -> Self {
        Self {
            version: WalVersion::V1,
            record_type,
            len,
            crc32,
            _reserved: [0u8; 7],
        }
    }

    /// Returns `None` for too-short input or unrecognised
    /// version byte.
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let version = WalVersion::try_from(buf[0]).ok()?;
        Some(Self {
            version,
            record_type: u16::from_le_bytes(
                [buf[2], buf[3]],
            ),
            len: u16::from_le_bytes(
                [buf[4], buf[5]],
            ),
            crc32: u32::from_le_bytes([
                buf[8], buf[9], buf[10], buf[11],
            ]),
            _reserved: [
                buf[1], buf[6], buf[7],
                buf[12], buf[13], buf[14], buf[15],
            ],
        })
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0] = self.version as u8;
        buf[2..4].copy_from_slice(
            &self.record_type.to_le_bytes(),
        );
        buf[4..6].copy_from_slice(
            &self.len.to_le_bytes(),
        );
        buf[8..12].copy_from_slice(
            &self.crc32.to_le_bytes(),
        );
        buf
    }
}
