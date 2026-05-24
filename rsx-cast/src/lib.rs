//! Log-backed reliable UDP transport (casting) + TCP cold-path
//! replication.
//!
//! Wire bytes = disk bytes = stream bytes. No serialization
//! step. NAK retransmits read from the WAL itself, so the
//! retransmit horizon is log retention, not buffer size.
//!
//! Transport-only; domain wire records live in `rsx-messages`
//! (or any consumer-defined crate). The transport accepts any
//! 16-byte-header + repr(C) payload that implements
//! [`CastRecord`].

pub mod header;
pub mod protocol;
pub mod encode_utils;
pub mod wal;
pub mod cast;
pub mod replication_server;
pub mod replication_client;
pub mod config;
pub mod tls;

pub use header::WalHeader;
pub use header::WalVersion;
pub use protocol::CastHeartbeat;
pub use protocol::CastRecord;
pub use protocol::CaughtUpRecord;
pub use protocol::Nak;
pub use protocol::ReplicationNotAvailable;
pub use protocol::RECORD_CAUGHT_UP;
pub use protocol::RECORD_HEARTBEAT;
pub use protocol::RECORD_NAK;
pub use protocol::RECORD_REPLICATION_NOT_AVAILABLE;
pub use encode_utils::as_bytes;
pub use encode_utils::compute_crc32;
pub use encode_utils::encode_record;
pub use wal::list_wal_files_across;
pub use wal::oldest_and_highest_seq;
pub use wal::read_record_at_seq;
pub use wal::RawWalRecord;
pub use wal::WalFileInfo;
pub use wal::WalReader;
pub use wal::WalWriter;
pub use replication_server::ReplicationService;
pub use replication_client::ReplicationConsumer;
pub use cast::CastRecv;
pub use cast::CastReceiver;
pub use cast::CastSender;
pub use config::CastConfig;
