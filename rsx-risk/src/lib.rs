//! Risk engine for RSX perpetuals exchange.
//!
//! One shard per user partition. Pre-trade margin checks,
//! post-trade position updates, funding settlement,
//! liquidation triggers. Postgres write-behind for
//! durability, advisory locks for single-writer guarantee.
//!
//! Receives orders from gateway via CMP/UDP, routes to
//! matching engines, processes fills back. DXS replay
//! for crash recovery from last persisted tip.

pub mod types;
pub mod position;
pub mod account;
pub mod margin;
pub mod price;
pub mod funding;
pub mod risk_utils;
pub mod config;
pub mod rings;
pub mod shard;
pub mod liquidation;
pub mod insurance;
pub mod persist;
pub mod replay;
pub mod schema;
pub mod lease;
pub mod replica;

pub use account::Account;
pub use config::LiquidationConfig;
pub use config::ReplicationConfig;
pub use config::ShardConfig;
pub use config::me_cmp_addrs_from_env;
pub use config::parse_me_cmp_addrs;
pub use funding::FundingConfig;
pub use insurance::InsuranceFund;
pub use lease::AdvisoryLease;
pub use margin::ExposureIndex;
pub use margin::MarginState;
pub use margin::PortfolioMargin;
pub use margin::SymbolRiskParams;
pub use persist::PersistEvent;
pub use replay::ColdStartState;
pub use position::Position;
pub use price::IndexPrice;
pub use replica::ReplicaPromotion;
pub use replica::ReplicaState;
pub use rings::OrderResponse;
pub use rings::ShardRings;
pub use shard::RiskShard;
pub use types::BboUpdate;
pub use types::FillEvent;
pub use types::OrderRequest;
pub use types::RejectReason;
