//! Shared test support for the TUI: a headless harness driving `App` +
//! `GatewayConn` + a `TestBackend` terminal.
//!
//! Each `tests/*.rs` file is its own crate, so this directory is pulled
//! in with `mod support;` and used via `support::harness::TuiHarness`.

pub mod harness;
