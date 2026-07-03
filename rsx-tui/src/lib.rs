//! RSX trading terminal — testable core.
//!
//! The binary (`main.rs`) owns the terminal + event loop; this library
//! owns UI state, event folding, key handling, the gateway transport
//! abstraction, and rendering — all drivable headless against a
//! `TestBackend` + `MockConn`. The live transport is a QUIC client
//! (webproto 49, user-facing only); the internal casting path is
//! separate and untouched.

pub mod app;
pub mod conn;
pub mod input;
pub mod quic;
pub mod render;
pub mod wire;

pub use app::drain;
pub use app::App;
pub use app::Field;
pub use app::OrderEntry;
pub use conn::GatewayConn;
pub use conn::GwEvent;
pub use conn::MockConn;
pub use conn::OrderReq;
pub use conn::Side;
pub use conn::Tif;
pub use quic::QuicConn;
pub use input::handle_key;
pub use input::Control;
pub use render::draw;

/// A scripted offline demo feed — the events `main` seeds into a
/// `MockConn` so `cargo run -p rsx-tui` shows a live-looking book
/// before the QUIC client is wired. Also handy in tests.
pub fn demo_events() -> Vec<GwEvent> {
    vec![
        GwEvent::Connected,
        GwEvent::Book {
            bids: vec![
                (10_000, 7),
                (9_999, 15),
                (9_998, 9),
                (9_997, 30),
            ],
            asks: vec![
                (10_001, 5),
                (10_002, 20),
                (10_003, 8),
                (10_004, 12),
            ],
        },
        GwEvent::Trade { side: Side::Buy, px: 10_001, qty: 5 },
        GwEvent::Trade { side: Side::Sell, px: 10_000, qty: 3 },
        GwEvent::Position {
            symbol: "PENGU-PERP",
            net_qty: 14,
            entry_px: 9_998,
            upnl: 42,
        },
    ]
}
