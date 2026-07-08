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
use rsx_tui::quic::roots;
use rsx_tui::quic::QuicConn;
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

/// Resolve the gateway transport from the environment.
///
/// `RSX_GW_ADDR` unset (or `mock`) → the offline demo feed (a `MockConn`
/// seeded with `demo_events`): there is no production QUIC gateway
/// listener yet, so a bare `rsx-tui` shows a live-looking book with
/// nothing running. `RSX_GW_ADDR=<ip:port>` → a real protobuf-over-QUIC
/// dial to that gateway, trusting the DER certificate at `RSX_GW_CERT`
/// and validating the TLS name `RSX_GW_SERVER_NAME` (default `localhost`).
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
    app.set_endpoint(&format!("quic://{server_addr}"));
    Ok(Box::new(QuicConn::connect(
        server_addr,
        server_name,
        store,
    )?))
}
