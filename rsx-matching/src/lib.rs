//! Matching engine tile logic for RSX.
//!
//! One instance per symbol, single-threaded, pinned to a
//! dedicated core. Receives orders via CMP/UDP from risk,
//! matches against [`rsx_book::Orderbook`], emits fills
//! and BBO updates to WAL.
//!
//! Dedup via [`dedup::DedupTracker`], wire decode via
//! [`wire`], WAL persistence via [`wal_integration`].

pub mod config;
pub mod dedup;
pub mod wal_integration;
pub mod wire;
