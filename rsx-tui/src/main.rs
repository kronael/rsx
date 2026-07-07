//! RSX trading terminal (ratatui) — binary entrypoint.
//!
//! Owns the terminal + event loop only. State, rendering, input, and
//! the gateway transport live in the `rsx_tui` library so they run
//! under a `TestBackend` in tests. Until the QUIC client lands this
//! drives a `MockConn` seeded with a demo feed. Run: `cargo run -p
//! rsx-tui`.

use ratatui::crossterm::event;
use ratatui::crossterm::event::Event;
use ratatui::Terminal;
use rsx_tui::app::App;
use rsx_tui::conn::MockConn;
use rsx_tui::demo_events;
use rsx_tui::drain;
use rsx_tui::draw;
use rsx_tui::handle_key;
use rsx_tui::Control;
use rsx_tui::GatewayConn;
use rsx_tui::WsConn;
use std::io;
use std::time::Duration;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

/// Event loop: drain gateway events, redraw, then block up to 100ms
/// for a key. `handle_key` mutates state / submits and says when to
/// quit.
fn run<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>) -> io::Result<()> {
    let mut app = App::new("PENGU-PERP");

    // One knob: RSX_GW_URL. Unset → the hosted deployment; `mock` → the
    // offline demo feed; any `ws://`/`wss://` → that gateway (e.g. a
    // local cluster at ws://127.0.0.1:8080). Trades as RSX_TUI_USER
    // (default 1) on PENGU; JWT minted with RSX_GW_JWT_SECRET.
    let mut conn: Box<dyn GatewayConn> = match resolve_url() {
        Some(url) => {
            app.set_endpoint(&url);
            let user_id = env_u32("RSX_TUI_USER", 1);
            let token = rsx_tui::ws::mint_jwt(user_id, &rsx_tui::ws::jwt_secret());
            Box::new(WsConn::connect(url, token, PENGU)?)
        }
        None => {
            let mut mock = MockConn::new();
            mock.push_events(demo_events());
            app.set_endpoint("mock://demo");
            Box::new(mock)
        }
    };

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

/// Hosted deployment — the default target. Override with `RSX_GW_URL`.
const DEPLOYMENT_URL: &str = "wss://rsx.krons.cx";

/// PENGU-PERP symbol id — the TUI's single market.
const PENGU: u32 = 10;

/// Resolve the gateway URL from `RSX_GW_URL`, or `None` for the offline
/// mock. Unset → the deployment; `mock` → demo; else → the given URL.
fn resolve_url() -> Option<String> {
    match std::env::var("RSX_GW_URL") {
        Ok(u) if u == "mock" => None,
        Ok(u) => Some(u),
        Err(_) => Some(DEPLOYMENT_URL.to_owned()),
    }
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}
