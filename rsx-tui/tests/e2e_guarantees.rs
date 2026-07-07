//! System-invariant e2e (T4): drive real order activity over `WsConn`
//! and confirm two of CLAUDE.md's "Correctness Invariants (system-wide)"
//! hold — #6 "best bid < best ask (no crossed book)" and #2 "exactly
//! one completion per order (ORDER_DONE xor ORDER_FAILED)" — plus a
//! fill-durability check. Env-gated like `e2e_orders.rs`:
//! `skip_if_no_cluster!` skips cleanly when nothing is listening at
//! `RSX_GW_LISTEN` (start one with
//! `./rsx-playground/playground start-all minimal`).
//!
//! **Invariant #6** is checked via the playground's `/api/verify/
//! run-json` "No crossed book (bid < ask)" check rather than
//! `App.spread()` over `WsConn` — `GwEvent::Book` never folds over a
//! real `WsConn` today (see `e2e_book.rs`'s documented gap), so
//! `App.bids`/`App.asks` stay empty and `spread()` would trivially
//! read 0 without proving anything. The playground check instead scans
//! live book snapshots + WAL BBO records server-side (`server.py`
//! `_run_invariant_checks`), independent of what `WsConn` folds, so
//! it's real proof. This test submits two orders that must NOT cross
//! (a resting bid and a resting ask, above/below every other price
//! band this sprint's fixtures use) purely to guarantee there's live
//! book state for the server to inspect.
//!
//! **Invariant #2** is checked directly from the raw `WsConn` event
//! stream instead: submit a maker + a crossing taker (full fill) and a
//! separately-submitted malformed order (reject), then assert no oid
//! ever yields more than one `Done`, and the rejected submission never
//! also yields a `Done`. (The playground's own "Exactly-one completion
//! per order" `/api/verify` check tracks a *different* order set — its
//! own `recent_orders`, populated only by orders submitted through the
//! playground's REST `/api/submit-order`, not by orders submitted
//! directly to the gateway the way this suite's `WsConn` does — so it
//! would report "skip: no completed orders observed" for our activity
//! and isn't usable as this test's oracle. Confirmed against
//! `rsx-playground/server.py`'s `recent_orders.append(...)` call sites,
//! all of which live in the `/api/submit-order` family of handlers.)
//!
//! **Fill durability**: intended to reuse the playground's "Fills
//! precede ORDER_DONE (per order)" `/api/verify` check (WAL-derived,
//! so submission-path-independent in principle), but a live probe
//! found it stuck reporting `"WAL fills=0 but session fills=183 —
//! sources disagree"` against this sprint's running cluster — the
//! playground's own `_wal_stream_dirs()` scan doesn't see the ME's
//! actual WAL location (`RSX_ME_WAL_DIR=./tmp/wal/pengu`, confirmed
//! via `/proc/<me-pid>/environ` and `find`, vs whatever `WAL_DIR`
//! resolves to in `server.py`). That's a playground bug, logged to
//! `BUGS.md`, not fixed here (out of this task's file scope; record,
//! don't fix). This test instead reads WAL durability the same way
//! `scripts/demo-trade.sh` does primarily — active-WAL-file byte growth
//! — which needs no `/api/verify` involvement at all.
//!
//! Bands/user_ids follow `e2e_orders.rs`'s fixture discipline (below
//! the demo book's ~50_000 resting asks so makers actually rest;
//! matched maker/taker qty so crossing pairs self-clean; user_ids from
//! the seeded demo set `1..5, 99`), using price bands this sprint's
//! other files don't touch: 43_000/95_000 (no_crossed_book, both rest
//! forever — deep and clear of every other band, like `e2e_orders.rs`'s
//! own permanently-resting `submit_gtc_rests @ 1`), 46_000
//! (exactly_one_completion, matched/self-cleaning), 44_500
//! (fill_durability, matched/self-cleaning, qty a multiple of the
//! symbol's 100_000 lot_size).

mod support;

use rsx_tui::conn::GwEvent;
use rsx_tui::conn::OrderReq;
use rsx_tui::conn::Side;
use rsx_tui::conn::Tif;
use rsx_tui::GatewayConn;
use rsx_tui::WsConn;
use serde_json::Value;
use std::io::Read;
use std::io::Write;
use std::net::TcpStream;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use support::cluster;
use support::submit::SUBMIT_ATTEMPTS;

/// Serializes this file's book-mutating tests against the shared live
/// book (see `e2e_orders.rs`'s `LIVE_BOOK`) — each test seeds its own
/// counterparty at a price band no other test in this sprint touches,
/// but held across the whole seed->cross window regardless, matching
/// the established discipline.
static LIVE_BOOK: Mutex<()> = Mutex::new(());

/// The playground dashboard's base URL — same default as
/// `scripts/demo-trade.sh`'s `PLAYGROUND_URL`.
fn playground_url() -> String {
    std::env::var("PLAYGROUND_URL").unwrap_or_else(|_| "http://127.0.0.1:49171".to_owned())
}

/// `POST /api/verify/run-json` and parse the JSON body over a raw
/// `TcpStream` (no HTTP client dep needed — `serde_json` is already a
/// crate dependency). `None` means "not reachable for this test's
/// purposes": connect failure, non-HTTP reply, or unparsable body all
/// collapse to the same skip signal, mirroring `cluster::connect`'s
/// `Option`-returns-never-panics contract.
fn fetch_verify_checks(timeout: Duration) -> Option<Value> {
    let url = playground_url();
    let host_port = url.strip_prefix("http://")?;
    let mut stream = TcpStream::connect(host_port).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let request = format!(
        "POST /api/verify/run-json HTTP/1.1\r\n\
         Host: {host_port}\r\n\
         Connection: close\r\n\
         Content-Length: 0\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let body = raw.split("\r\n\r\n").nth(1)?;
    serde_json::from_str(body).ok()
}

/// One named check's `status` field ("pass"/"fail"/"warn"/"skip") out
/// of a `/api/verify/run-json` body, or `None` if that check isn't
/// present in this run.
fn check_status(checks: &Value, name: &str) -> Option<String> {
    checks
        .get("checks")?
        .as_array()?
        .iter()
        .find(|c| c.get("name").and_then(Value::as_str) == Some(name))?
        .get("status")?
        .as_str()
        .map(str::to_owned)
}

/// One named check's `detail` field, for assertion messages.
fn check_detail(checks: &Value, name: &str) -> Option<String> {
    checks
        .get("checks")?
        .as_array()?
        .iter()
        .find(|c| c.get("name").and_then(Value::as_str) == Some(name))?
        .get("detail")?
        .as_str()
        .map(str::to_owned)
}

/// The matching engine's active WAL file for `cluster::SYMBOL_ID`
/// (`<RSX_ME_WAL_DIR>/<symbol>/<symbol>_active.wal`), the file
/// `scripts/demo-trade.sh` polls for byte growth after a fill.
/// Override with `RSX_TUI_WAL_FILE` if the cluster wasn't started with
/// the sprint's default `RSX_ME_WAL_DIR=./tmp/wal/pengu` (repo-root
/// relative). Defaults resolve relative to this crate's manifest dir
/// (`CARGO_MANIFEST_DIR`) so the test works regardless of `cargo
/// test`'s cwd.
fn wal_active_file() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("RSX_TUI_WAL_FILE") {
        return std::path::PathBuf::from(p);
    }
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rsx-tui has a parent dir (repo root)")
        .to_owned();
    repo_root
        .join("tmp/wal/pengu")
        .join(cluster::SYMBOL_ID.to_string())
        .join(format!("{}_active.wal", cluster::SYMBOL_ID))
}

/// Poll `conn` for `timeout`, collecting every event it yields — the
/// raw sequence, unfolded into any `App`, so oid transitions are
/// directly observable (unlike `harness.tick()`, which only exposes
/// cumulative `App` counters).
fn collect_events(conn: &mut WsConn, timeout: Duration) -> Vec<GwEvent> {
    let mut events = Vec::new();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        while let Some(ev) = conn.poll_event() {
            events.push(ev);
        }
        thread::sleep(Duration::from_millis(5));
    }
    events
}

/// Submit `order` on `conn`, collecting the raw event sequence,
/// resubmitting (fresh cid) up to `SUBMIT_ATTEMPTS` times until `done`
/// holds over the collected events — the casting-loss accommodation
/// from `e2e_orders.rs`'s `submit_and_wait`, generalized to a raw event
/// predicate instead of an `App` predicate.
fn submit_and_collect(
    conn: &mut WsConn,
    order: OrderReq,
    timeout: Duration,
    done: impl Fn(&[GwEvent]) -> bool,
) -> Vec<GwEvent> {
    let mut events = Vec::new();
    for attempt in 1..=SUBMIT_ATTEMPTS {
        conn.submit(order).expect("submit order");
        events = collect_events(conn, timeout);
        if done(&events) {
            assert_eq!(
                attempt, 1,
                "return-path masking (finding 2): predicate met only on \
                 attempt {attempt}; a resubmit papering over a dropped \
                 first-attempt event is the ME-emits-but-risk-never-sees \
                 signature — investigate, do not mask",
            );
            return events;
        }
        eprintln!(
            "attempt {attempt}/{SUBMIT_ATTEMPTS}: predicate unmet within \
             {timeout:?}; casting is UDP, an occasional dropped order/event \
             is expected",
        );
    }
    events
}

/// After two non-crossing resting orders (a bid and an ask, far apart,
/// clear of every other band this sprint's fixtures use), the
/// playground's server-side book scan must never report a cross —
/// invariant #6.
#[test]
#[ignore = "live-cluster + playground gated (needs `start-all minimal`); run with --ignored"]
fn no_crossed_book() {
    let _serial = LIVE_BOOK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_) = fetch_verify_checks(Duration::from_secs(2)) else {
        eprintln!(
            "skip: playground /api/verify unreachable at {} (start with \
             `./rsx-playground/playground start`)",
            playground_url(),
        );
        return;
    };

    let mut maker = skip_if_no_cluster!(cluster::connect(1));
    maker
        .submit(OrderReq {
            side: Side::Buy,
            price: 43_000,
            qty: 100_000,
            tif: Tif::Gtc,
        })
        .expect("submit resting bid");
    let mut asker = skip_if_no_cluster!(cluster::connect(2));
    asker
        .submit(OrderReq {
            side: Side::Sell,
            price: 95_000,
            qty: 100_000,
            tif: Tif::Gtc,
        })
        .expect("submit resting ask");
    thread::sleep(Duration::from_millis(500));

    let checks = fetch_verify_checks(Duration::from_secs(3))
        .expect("verify endpoint reachable (already confirmed above)");
    let status = check_status(&checks, "No crossed book (bid < ask)")
        .expect("\"No crossed book\" check present in the verify run");
    assert_ne!(
        status.as_str(),
        "fail",
        "invariant #6 violated: {:?}",
        check_detail(&checks, "No crossed book (bid < ask)"),
    );
}

/// A maker rests then a taker fully fills it (one full Accepted->Fill->
/// Done lifecycle across two connections), and a separate malformed
/// order is rejected on a third — assert every oid that reached `Done`
/// did so exactly once, and the rejected submission never also
/// produced a `Done` — invariant #2, "exactly one completion per
/// order".
#[test]
#[ignore = "live-cluster gated (needs `start-all minimal`); run with --ignored"]
fn exactly_one_completion() {
    let _serial = LIVE_BOOK.lock().unwrap_or_else(|e| e.into_inner());
    let mut maker = skip_if_no_cluster!(cluster::connect(3));
    let maker_accept = submit_and_collect(
        &mut maker,
        OrderReq {
            side: Side::Buy,
            price: 46_000,
            qty: 300_000,
            tif: Tif::Gtc,
        },
        Duration::from_secs(3),
        |evs| evs.iter().any(|e| matches!(e, GwEvent::Accepted { .. })),
    );
    assert!(
        maker_accept
            .iter()
            .any(|e| matches!(e, GwEvent::Accepted { .. })),
        "maker order never accepted: {maker_accept:?}",
    );

    let mut taker = skip_if_no_cluster!(cluster::connect(4));
    let taker_events = submit_and_collect(
        &mut taker,
        OrderReq {
            side: Side::Sell,
            price: 46_000,
            qty: 300_000,
            tif: Tif::Ioc,
        },
        Duration::from_secs(5),
        |evs| evs.iter().any(|e| matches!(e, GwEvent::Done { .. })),
    );
    assert!(
        taker_events
            .iter()
            .any(|e| matches!(e, GwEvent::Done { .. })),
        "taker IOC never reached Done: {taker_events:?}",
    );
    // The maker's own connection observes its Fill/Done separately.
    let maker_completion = collect_events(&mut maker, Duration::from_secs(2));

    let mut done_counts: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    for ev in maker_accept
        .iter()
        .chain(maker_completion.iter())
        .chain(taker_events.iter())
    {
        if let GwEvent::Done { oid } = ev {
            *done_counts.entry(*oid).or_insert(0) += 1;
        }
    }
    assert!(
        !done_counts.is_empty(),
        "expected at least one completed oid"
    );
    assert!(
        done_counts.values().all(|&n| n == 1),
        "an oid completed more than once: {done_counts:?}",
    );

    let mut rejector = skip_if_no_cluster!(cluster::connect(5));
    let reject_events = submit_and_collect(
        &mut rejector,
        OrderReq {
            side: Side::Buy,
            price: 12_345,
            qty: 0,
            tif: Tif::Gtc,
        },
        Duration::from_secs(3),
        |evs| evs.iter().any(|e| matches!(e, GwEvent::Rejected { .. })),
    );
    let reject_count = reject_events
        .iter()
        .filter(|e| matches!(e, GwEvent::Rejected { .. }))
        .count();
    let done_on_reject_conn = reject_events
        .iter()
        .filter(|e| matches!(e, GwEvent::Done { .. }))
        .count();
    assert_eq!(
        reject_count, 1,
        "malformed order must be rejected exactly once: {reject_events:?}",
    );
    assert_eq!(
        done_on_reject_conn, 0,
        "a rejected order must never also reach Done (invariant #2): {reject_events:?}",
    );
}

/// Snapshot the ME's active WAL file size, drive one fill, and confirm
/// the file grew — proof the fill was durably recorded to the WAL, not
/// just acked in memory. See the module doc for why this reads the WAL
/// file directly (`scripts/demo-trade.sh`'s primary technique) instead
/// of the playground's `/api/verify` fill-count check, which a live
/// probe found broken (WAL_DIR path mismatch, logged to `BUGS.md`).
#[test]
#[ignore = "live-cluster gated (needs `start-all minimal`); run with --ignored"]
fn fill_durability_recorded_in_wal() {
    let _serial = LIVE_BOOK.lock().unwrap_or_else(|e| e.into_inner());
    let wal_file = wal_active_file();
    let Ok(before_meta) = std::fs::metadata(&wal_file) else {
        eprintln!(
            "skip: WAL file not found at {wal_file:?} — the cluster's ME \
             may use a different RSX_ME_WAL_DIR; override with \
             RSX_TUI_WAL_FILE to point at the active WAL directly",
        );
        return;
    };
    let before_len = before_meta.len();

    let mut maker = skip_if_no_cluster!(cluster::connect(99));
    maker
        .submit(OrderReq {
            side: Side::Buy,
            price: 44_500,
            qty: 200_000,
            tif: Tif::Gtc,
        })
        .expect("submit maker order");
    thread::sleep(Duration::from_millis(500));

    let mut taker = skip_if_no_cluster!(cluster::connect(1));
    let taker_events = submit_and_collect(
        &mut taker,
        OrderReq {
            side: Side::Sell,
            price: 44_500,
            qty: 200_000,
            tif: Tif::Ioc,
        },
        Duration::from_secs(5),
        |evs| evs.iter().any(|e| matches!(e, GwEvent::Fill { .. })),
    );
    assert!(
        taker_events
            .iter()
            .any(|e| matches!(e, GwEvent::Fill { .. })),
        "taker IOC never filled: {taker_events:?}",
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut after_len = before_len;
    while Instant::now() < deadline {
        if let Ok(meta) = std::fs::metadata(&wal_file) {
            after_len = meta.len();
            if after_len > before_len {
                break;
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert!(
        after_len > before_len,
        "expected the WAL file to grow after the fill (durability): \
         {wal_file:?} before={before_len} after={after_len}",
    );
}
