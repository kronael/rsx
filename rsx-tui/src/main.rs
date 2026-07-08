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
use rustls::RootCertStore;
use std::io;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
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

/// The production gateway. Run `rsx-tui` with no config and it dials this.
const PROD_HOST: &str = "rsx.krons.cx";
const PROD_PORT: u16 = 4433;
/// The local debug gateway (`RSX_GW_LOCAL=1`).
const LOCAL_ADDR: &str = "127.0.0.1:4433";
/// Default DER cert for the local debug gateway (override with `RSX_GW_CERT`).
const DEFAULT_LOCAL_CERT: &str = "gateway.der";

/// Which gateway `rsx-tui` dials. Simple by default: run it and it dials
/// the production server; override for local work or the offline demo.
///
/// - default (no env) → **`rsx.krons.cx`**, trusting the system's real CA
///   roots — no cert file needed.
/// - `RSX_GW_LOCAL=1` → a local debug gateway on `127.0.0.1:4433`, trusting
///   the DER cert at `RSX_GW_CERT` (default `gateway.der`), TLS name
///   `localhost` — the shape the local testing deployment produces.
/// - `RSX_GW_ADDR=mock` → the offline demo feed (no server needed).
/// - `RSX_GW_ADDR=<ip:port>` (+ `RSX_GW_CERT`) → an explicit pinned dial.
///
/// Identity: `RSX_TUI_USER` is minted into an HS256 JWT (`RSX_GW_JWT_SECRET`)
/// sent in the auth first-frame; `RSX_TUI_SYMBOL` stamps every order. (No
/// gateway QUIC listener answers this wire yet — the dial is real, the
/// server is the pending roadmap step.)
fn connect(app: &mut App) -> io::Result<Box<dyn GatewayConn>> {
    let gw = std::env::var("RSX_GW_ADDR").ok();

    // Offline demo: no server needed.
    if gw.as_deref() == Some("mock") {
        let mut mock = MockConn::new();
        mock.push_events(demo_events());
        app.set_endpoint("mock://demo");
        return Ok(Box::new(mock));
    }

    let local = gw.as_deref() == Some("local") || std::env::var("RSX_GW_LOCAL").is_ok();

    let (server_addr, server_name, store): (SocketAddr, String, RootCertStore) =
        if let Some(explicit) = gw.filter(|a| a.as_str() != "local") {
            // Explicit pinned dial (advanced): ip:port + RSX_GW_CERT.
            let addr = explicit.parse().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidInput, format!("RSX_GW_ADDR: {e}"))
            })?;
            let cert = std::env::var("RSX_GW_CERT").map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "RSX_GW_ADDR is set but RSX_GW_CERT (path to the gateway DER cert) is not",
                )
            })?;
            let name =
                std::env::var("RSX_GW_SERVER_NAME").unwrap_or_else(|_| "localhost".to_owned());
            (
                addr,
                name,
                roots([CertificateDer::from(std::fs::read(&cert)?)])?,
            )
        } else if local {
            // Local debug gateway: localhost + pinned dev cert.
            let addr = LOCAL_ADDR
                .parse()
                .expect("LOCAL_ADDR is a valid SocketAddr");
            let cert =
                std::env::var("RSX_GW_CERT").unwrap_or_else(|_| DEFAULT_LOCAL_CERT.to_owned());
            (
                addr,
                "localhost".to_owned(),
                roots([CertificateDer::from(std::fs::read(&cert)?)])?,
            )
        } else {
            // Default: the production server, trusting real CA roots.
            let addr = (PROD_HOST, PROD_PORT)
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{PROD_HOST} did not resolve"),
                    )
                })?;
            (addr, PROD_HOST.to_owned(), prod_roots())
        };

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

/// The system's real CA roots (webpki-roots) — used for the production dial
/// so no pinned cert file is needed.
fn prod_roots() -> RootCertStore {
    let mut store = RootCertStore::empty();
    store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    store
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
