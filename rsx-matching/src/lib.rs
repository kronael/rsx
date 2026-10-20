//! rsx-matching: matching engine tile logic. See ARCHITECTURE.md and `specs/2/21-orderbook.md`.

pub mod checkpoint;
pub mod config;
pub mod dedup;
pub mod metrics;
pub mod recovery;
pub mod wal;
pub mod wire;
