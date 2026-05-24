//! Log-backed reliable UDP transport (CMP) + TCP cold-path
//! replay (DXS).
//!
//! Wire bytes = disk bytes = stream bytes. No serialization
//! step. NAK retransmits read from the WAL itself, so the
//! retransmit horizon is log retention, not buffer size.
//!
//! Transport-only; domain wire records live in `rsx-messages`
//! (or any consumer-defined crate). The transport accepts any
//! 16-byte-header + repr(C) payload that implements
//! [`CmpRecord`].

pub mod header;
pub mod protocol;
pub mod encode_utils;
pub mod wal;
pub mod cmp;
pub mod server;
pub mod client;
pub mod config;
pub mod tls;

pub use protocol as records;

pub use header::WalHeader;
pub use protocol::CmpHeartbeat;
pub use protocol::CmpRecord;
pub use protocol::CaughtUpRecord;
pub use protocol::Nak;
pub use protocol::StatusMessage;
pub use protocol::RECORD_CAUGHT_UP;
pub use protocol::RECORD_HEARTBEAT;
pub use protocol::RECORD_NAK;
pub use protocol::RECORD_STATUS_MESSAGE;
pub use encode_utils::as_bytes;
pub use encode_utils::compute_crc32;
pub use encode_utils::encode_record;
pub use wal::read_record_at_seq;
pub use wal::RawWalRecord;
pub use wal::WalReader;
pub use wal::WalWriter;
pub use server::DxsReplayService;
pub use client::DxsConsumer;
pub use cmp::CmpReceiver;
pub use cmp::CmpSender;
pub use config::CmpConfig;
