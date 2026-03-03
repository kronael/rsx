//! Shared orderbook for the RSX matching engine.
//!
//! Slab-allocated price-time FIFO book with compressed
//! price indexing. Zero heap on the hot path. Each ME
//! instance owns one [`Orderbook`] per symbol.
//!
//! Key types: [`Slab`] (arena allocator), [`CompressionMap`]
//! (sparse price→index), [`PriceLevel`] (linked list per
//! price), [`OrderSlot`] (128B, cache-line aligned),
//! [`Event`] (fill/done/cancel output buffer).

pub mod slab;
pub mod compression;
pub mod level;
pub mod order;
pub mod event;
pub mod user;
pub mod book;
pub mod matching;
pub mod migration;
pub mod snapshot;

pub use book::BookState;
pub use book::Orderbook;
pub use compression::CompressionMap;
pub use event::Event;
pub use level::PriceLevel;
pub use order::OrderSlot;
pub use slab::Slab;
pub use user::UserState;
pub use user::try_reclaim;
pub use user::RECLAIM_GRACE_NS;
