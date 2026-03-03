//! WebSocket gateway for RSX perpetuals exchange.
//!
//! Bridges external WS clients to the internal CMP/UDP
//! transport. monoio (io_uring) for network I/O. JWT auth,
//! per-user rate limiting, circuit breaker, pending order
//! tracking with timeout. Stateless — crash and restart.

pub mod protocol;
pub mod convert;
pub mod types;
pub mod config;
pub mod rate_limit;
pub mod circuit;
pub mod pending;
pub mod order_id;
pub mod ws;
pub mod handler;
pub mod state;
pub mod jwt;
pub mod route;
pub mod rest;
