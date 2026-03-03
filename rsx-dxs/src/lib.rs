//! WAL, CMP, and DXS transport library for RSX.
//!
//! Three concerns: WAL persistence (write/read/rotate),
//! CMP protocol (UDP flow control with NACK), and DXS
//! replay (TCP streaming from sequence N). Disk format =
//! wire format = stream format — no transformation.
//!
//! 16B header + `#[repr(C, align(64))]` payload per record.
//! 15 record types covering fills, BBO, orders, marks,
//! liquidations, and config events.

pub mod header;
pub mod records;
pub mod encode_utils;
pub mod wal;
pub mod server;
pub mod client;
pub mod config;
pub mod cmp;

pub use header::*;
pub use records::*;
pub use encode_utils::*;
pub use wal::*;
pub use server::*;
pub use client::*;
pub use config::*;
pub use cmp::*;
