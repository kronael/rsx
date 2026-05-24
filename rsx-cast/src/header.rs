/// 16-byte WAL record header. See `specs/48-wal.md` for field layout and version policy.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalHeader {
    pub version: u8,
    pub _pad0: u8,
    pub record_type: u16,
    pub len: u16,
    pub _pad1: u16,
    pub crc32: u32,
    pub _reserved: [u8; 4],
}

const _: () = assert!(
    std::mem::size_of::<WalHeader>() == 16,
    "WalHeader must be exactly 16 bytes",
);

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
            version: WalVersion::V1 as u8,
            _pad0: 0,
            record_type,
            len,
            _pad1: 0,
            crc32,
            _reserved: [0u8; 4],
        }
    }

    /// Returns `None` for too-short input or unrecognised
    /// version byte.
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        WalVersion::try_from(buf[0]).ok()?;
        Some(unsafe {
            std::ptr::read_unaligned(
                buf.as_ptr() as *const Self,
            )
        })
    }

    pub fn to_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self as *const Self as *const u8,
                Self::SIZE,
            )
        }
    }
}
