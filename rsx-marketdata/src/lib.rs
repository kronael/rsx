//! Market data service for RSX.
//!
//! Maintains a shadow orderbook from ME WAL events
//! (INSERT/CANCEL/FILL). Broadcasts L2 depth, BBO, and
//! trade tape to public WS subscribers. monoio (io_uring)
//! for network I/O. Seq gap detection triggers resync.

pub mod shadow;
pub mod types;
pub mod subscription;
pub mod protocol;
pub mod config;
pub mod handler;
pub mod state;
pub mod ws;
pub mod replay;
