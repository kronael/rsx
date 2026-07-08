//! RSX trading terminal (ratatui) — binary entrypoint.
//!
//! Owns the terminal + event loop only. State, rendering, input, and the
//! gateway transport live in the `rsx_tui` library so they run under a
//! `TestBackend` in tests. The live transport is protobuf-over-QUIC
//! (`QuicConn`); with no gateway configured it drives a `MockConn` seeded
//! with a demo feed. Run: `cargo run -p rsx-tui`.

use ratatui::crossterm::event;
use ratatui::crossterm::event::Event;
use ratatui::Terminal;
use rsx_tui::app::App;
use rsx_tui::conn::MockConn;
use rsx_tui::demo_events;
use rsx_tui::drain;
use rsx_tui::draw;
use rsx_tui::handle_key;
use rsx_tui::quic::mint_jwt;
use rsx_tui::quic::roots;
use rsx_tui::quic::QuicConn;
use rsx_tui::quic::Session;
use rsx_tui::Control;
use rsx_tui::GatewayConn;
use rustls::pki_types::CertificateDer;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

/// Event loop: drain gateway events, redraw, then block up to 100ms for a
/// key. `handle_key` mutates state / submits and says when to quit.
fn run<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>) -> io::Result<()> {
    let mut app = App::new(SYMBOL);
    let mut conn = connect(&mut app)?;

    loop {
        drain(&mut app, conn.as_mut());
        terminal.draw(|f| draw(f, &app))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if handle_key(&mut app, key.code, conn.as_mut()) == Control::Quit {
                return Ok(());
            }
        }
    }
}

/// PENGU-PERP — the TUI's single market.
const SYMBOL: &str = "PENGU-PERP";

/// Dev JWT signing secret when `RSX_GW_JWT_SECRET` is unset — matches
/// the gateway's dev default. Never a production value.
const DEFAULT_JWT_SECRET: &str = "rsx-dev-secret-not-for-prod-padpad";

/// Resolve the gateway transport from the environment.
///
/// `RSX_GW_ADDR` unset (or `mock`) → the offline demo feed (a `MockConn`
/// seeded with `demo_events`): there is no production QUIC gateway
/// listener yet, so a bare `rsx-tui` shows a live-looking book with
/// nothing running. `RSX_GW_ADDR=<ip:port>` → a real protobuf-over-QUIC
/// dial to that gateway, trusting the DER certificate at `RSX_GW_CERT`
/// and validating the TLS name `RSX_GW_SERVER_NAME` (default `localhost`).
///
/// Identity for the live dial: `RSX_TUI_USER` (the u32 the session trades
/// as) is minted into an HS256 JWT with `RSX_GW_JWT_SECRET` and sent in
/// the auth first-frame; `RSX_TUI_SYMBOL` (the u32 symbol id) stamps every
/// order.
fn connect(app: &mut App) -> io::Result<Box<dyn GatewayConn>> {
    let addr = match std::env::var("RSX_GW_ADDR") {
        Ok(a) if a != "mock" => a,
        _ => {
            let mut mock = MockConn::new();
            mock.push_events(demo_events());
            app.set_endpoint("mock://demo");
            return Ok(Box::new(mock));
        }
    };
    let server_addr: SocketAddr = addr
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("RSX_GW_ADDR: {e}")))?;
    let cert_path = std::env::var("RSX_GW_CERT").map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "RSX_GW_ADDR is set but RSX_GW_CERT (path to the gateway's DER cert) is not",
        )
    })?;
    let server_name =
        std::env::var("RSX_GW_SERVER_NAME").unwrap_or_else(|_| "localhost".to_owned());
    let der = CertificateDer::from(std::fs::read(&cert_path)?);
    let store = roots([der])?;
    let symbol_id = env_u32("RSX_TUI_SYMBOL", 0)?;
    let user = env_u32("RSX_TUI_USER", 0)?;
    let secret =
        std::env::var("RSX_GW_JWT_SECRET").unwrap_or_else(|_| DEFAULT_JWT_SECRET.to_owned());
    let jwt = mint_jwt(user, &secret);
    app.set_endpoint(&format!("quic://{server_addr}"));
    Ok(Box::new(QuicConn::connect(
        server_addr,
        server_name,
        store,
        Session {
            symbol_id,
            user,
            jwt,
        },
    )?))
}

/// Parse a `u32` environment variable, or `default` when unset. A
/// malformed value is a hard error — a typo'd id must not silently
/// become the default and trade as the wrong user or symbol.
fn env_u32(key: &str, default: u32) -> io::Result<u32> {
    match std::env::var(key) {
        Ok(v) => v
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("{key}: {e}"))),
        Err(_) => Ok(default),
    }
}
