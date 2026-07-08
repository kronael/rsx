//! rsx-marketdata: market data service. See ARCHITECTURE.md and `specs/2/16-marketdata.md`.

pub mod config;
pub mod egress;
pub mod handler;
pub mod records;
pub mod replay;
pub mod shadow;
pub mod state;
pub mod subscription;
pub mod types;
pub mod ws;
