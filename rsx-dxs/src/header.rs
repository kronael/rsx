/// WAL record header (16 bytes exactly).
///
/// Layout (LE on the wire):
/// ```text
/// offset 0..2   record_type: u16
/// offset 2..4   len:         u16   (payload length, bytes)
/// offset 4..8   crc32:       u32   (CRC32C of payload)
/// offset 8      version:     u8    (wire-format version)
/// offset 9..16  _reserved:   [u8; 7]  (must be zero)
/// ```
///
/// `version` repurposes one of the previously-reserved 8
/// bytes to give the protocol a coordinated upgrade path
/// without changing the on-the-wire size.
///
/// - `0` (`WAL_HEADER_VERSION_V0`): the original, pre-2026
///   layout where bytes 8..16 were all-zero. Accepted on
///   read for back-compat with existing WAL files; not
///   emitted by new writes.
/// - `1` (`WAL_HEADER_VERSION_V1`): the current layout
///   declared by this struct. All new writes carry this.
///
/// Adding a new record type does NOT bump the version (record
/// types are additive — the spec promises that). Bumping the
/// version is reserved for changes that would break a v1
/// reader (e.g., re-laying out the header, changing CRC
/// algorithm). A coordinated stop-redeploy is required across
/// senders and receivers when bumping.
#[derive(Debug, Clone, Copy)]
pub struct WalHeader {
    pub record_type: u16,
    pub len: u16,
    pub crc32: u32,
    pub version: u8,
    pub _reserved: [u8; 7],
}

/// Legacy wire format (pre-version-byte). Reserved bytes
/// were all zero. Accepted on read; never emitted.
pub const WAL_HEADER_VERSION_V0: u8 = 0;

/// Current wire format. Emitted by all new writes.
pub const WAL_HEADER_VERSION_V1: u8 = 1;

/// The version this binary writes. Single source of truth.
pub const WAL_HEADER_VERSION_LATEST: u8 =
    WAL_HEADER_VERSION_V1;

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
            version: WAL_HEADER_VERSION_LATEST,
            _reserved: [0u8; 7],
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
        buf[8] = self.version;
        // bytes 9..16 stay zeroed (_reserved)
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
            version: buf[8],
            _reserved: [
                buf[9], buf[10], buf[11], buf[12],
                buf[13], buf[14], buf[15],
            ],
        })
    }

    /// Returns true if this binary can decode the payload
    /// behind a header with this version.
    ///
    /// Accepts `V0` (legacy zero) and `V1` (current). Any
    /// other version is rejected — the receiver must not
    /// attempt to interpret payload bytes whose framing it
    /// doesn't understand.
    pub fn is_supported_version(&self) -> bool {
        matches!(
            self.version,
            WAL_HEADER_VERSION_V0
                | WAL_HEADER_VERSION_V1
        )
    }
}
