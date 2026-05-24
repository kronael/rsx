//! rsx-marketdata: market data service. See ARCHITECTURE.md and `specs/2/16-marketdata.md`.

pub mod shadow;
pub mod types;
pub mod subscription;
pub mod records;
pub mod config;
pub mod handler;
pub mod state;
pub mod ws;
pub mod replay;
