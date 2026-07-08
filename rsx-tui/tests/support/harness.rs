//! `TuiHarness`: drives a full `App` + `GatewayConn` + `TestBackend`
//! session headlessly, the way `play_test.rs` does inline but shared
//! across every later test file. Owns exactly one terminal so
//! `screen()` and `wait_for` observe the same buffer a real session
//! would.

// Dead-code is evaluated per test-binary crate, and each including test
// exercises only the harness helpers it needs, so some read as dead there.
#![allow(dead_code)]

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;
use ratatui::Terminal;
use rsx_tui::app::App;
use rsx_tui::conn::GatewayConn;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::MockConn;
use rsx_tui::conn::OrderReq;
use rsx_tui::drain;
use rsx_tui::draw;
use rsx_tui::handle_key;
use rsx_tui::Control;
use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

/// Default test terminal size, matching `play_test.rs`/`render_test.rs`.
const COLS: u16 = 120;
const ROWS: u16 = 30;

/// Default symbol for a harness session (matches `App::new` usage
/// elsewhere in the crate and `demo-trade.sh`'s `SYMBOL_ID=10`).
pub const SYMBOL: &str = "PENGU-PERP";

/// Forwards to a `MockConn` shared with the harness's caller, so a
/// scripted test can keep pushing events into an already-boxed
/// connection. `GatewayConn` has no `Any`/downcast (conn.rs is out of
/// scope for this task), so this `Rc<RefCell<_>>` indirection is how
/// `new_mock()` hands the harness a `Box<dyn GatewayConn>` per the
/// spec while still letting `push_mock_events` reach the same queue.
struct SharedMock(Rc<RefCell<MockConn>>);

impl GatewayConn for SharedMock {
    fn submit(&mut self, order: OrderReq) -> io::Result<()> {
        self.0.borrow_mut().submit(order)
    }

    fn poll_event(&mut self) -> Option<GwEvent> {
        self.0.borrow_mut().poll_event()
    }
}

/// A headless TUI session: `App` state, a boxed `GatewayConn` (a
/// `MockConn` for unit-style tests, or any caller-supplied transport),
/// and a `TestBackend` terminal to render into.
pub struct TuiHarness {
    pub app: App,
    pub conn: Box<dyn GatewayConn>,
    terminal: Terminal<TestBackend>,
    /// Set only by `new_mock()`, so `push_mock_events` can reach the
    /// `MockConn` behind `conn` without a downcast.
    mock: Option<Rc<RefCell<MockConn>>>,
}

impl TuiHarness {
    /// A harness over a fresh `MockConn` — no network, the shape every
    /// smoke/unit-style test wants. Pair with `push_mock_events` to
    /// script inbound gateway events.
    pub fn new_mock() -> Self {
        let mock = Rc::new(RefCell::new(MockConn::new()));
        let conn: Box<dyn GatewayConn> = Box::new(SharedMock(mock.clone()));
        let mut harness = TuiHarness::new_with(conn);
        harness.mock = Some(mock);
        harness
    }

    /// A harness over a caller-supplied connection (e.g. a `QuicConn`).
    /// Draws once before returning so `screen()` reflects the initial
    /// (pre-`Connected`) state right away, without the caller needing a
    /// throwaway `tick()` first.
    pub fn new_with(conn: Box<dyn GatewayConn>) -> Self {
        let terminal =
            Terminal::new(TestBackend::new(COLS, ROWS)).expect("build TestBackend terminal");
        let mut harness = TuiHarness {
            app: App::new(SYMBOL),
            conn,
            terminal,
            mock: None,
        };
        harness.tick();
        harness
    }

    /// Queue events the `MockConn` behind this harness will yield on
    /// subsequent `tick`/`wait_for` polls. Panics if this harness was
    /// not built with `new_mock()` — a harness over a real transport
    /// observes the gateway itself, not this queue.
    pub fn push_mock_events(&mut self, events: impl IntoIterator<Item = GwEvent>) {
        let mock = self
            .mock
            .as_ref()
            .expect("push_mock_events requires a harness built with new_mock()");
        mock.borrow_mut().push_events(events);
    }

    /// Apply one key, mirroring a single keystroke in the real event
    /// loop, then drain + redraw so `screen()`/state reflect it.
    pub fn feed_key(&mut self, code: KeyCode) -> Control {
        let ctrl = handle_key(&mut self.app, code, self.conn.as_mut());
        self.tick();
        ctrl
    }

    /// Type a string one character at a time (digits, letters, or
    /// commands) via `feed_key` — the multi-key analog of
    /// `play_test.rs`'s `type_digits`.
    pub fn feed_str(&mut self, s: &str) {
        for c in s.chars() {
            self.feed_key(KeyCode::Char(c));
        }
    }

    /// Drain every pending gateway event into `App`, then redraw. One
    /// render tick, the same order the real event loop uses
    /// (`main.rs`: drain, then draw).
    pub fn tick(&mut self) {
        drain(&mut self.app, self.conn.as_mut());
        let app = &self.app;
        self.terminal.draw(|f| draw(f, app)).expect("draw");
    }

    /// Flatten the last-drawn buffer into one string of cell symbols
    /// (same flattening `play_test.rs`/`render_test.rs` use inline).
    pub fn screen(&self) -> String {
        self.terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Tick in a loop (draining the connection each pass) until `pred`
    /// holds or `timeout` elapses. Returns the elapsed time on success,
    /// `None` on timeout — the shape a test uses to wait for an
    /// `Accepted`/`Fill`/`Done` to arrive over a live transport, and a
    /// cheap way for `MockConn` tests to wait for something already
    /// queued.
    pub fn wait_for<F>(&mut self, pred: F, timeout: Duration) -> Option<Duration>
    where
        F: Fn(&App) -> bool,
    {
        let start = Instant::now();
        loop {
            self.tick();
            if pred(&self.app) {
                return Some(start.elapsed());
            }
            if start.elapsed() >= timeout {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Assert the current screen contains `needle`, panicking with the
    /// full screen text on failure (so a broken assertion is legible,
    /// not just "assertion failed").
    pub fn assert_screen(&self, needle: &str) {
        let screen = self.screen();
        assert!(
            screen.contains(needle),
            "expected screen to contain {needle:?}, got:\n{screen}",
        );
    }

    /// Assert a predicate over `App`, panicking with `msg` on failure.
    pub fn assert_state<F>(&self, msg: &str, pred: F)
    where
        F: FnOnce(&App) -> bool,
    {
        assert!(pred(&self.app), "assert_state failed: {msg}");
    }
}
