//! Shared test support for the TUI e2e/bench suites (T3-T5): a
//! headless harness driving `App` + `GatewayConn` + a `TestBackend`
//! terminal, a live-cluster dialer, and timing helpers for the
//! latency comparisons.
//!
//! Each `tests/*.rs` file is its own crate, so this directory is
//! pulled in with `#[path = "support/mod.rs"] mod support;` (or plain
//! `mod support;`, which resolves the same way) and used via
//! `support::harness::TuiHarness`, etc.

pub mod cluster;
pub mod harness;
pub mod submit;
pub mod timing;
