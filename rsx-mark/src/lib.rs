//! Mark price aggregator for RSX.
//!
//! Connects to external feeds (Binance, Coinbase WS),
//! computes weighted median, publishes MARK_PRICE records
//! to WAL. Risk engine consumes these for margin/liquidation
//! calculations. Staleness filter drops stale sources after
//! configurable timeout.

pub mod aggregator;
pub mod config;
pub mod source;
pub mod types;
