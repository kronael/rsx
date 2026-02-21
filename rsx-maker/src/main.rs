use rsx_types::install_panic_handler;
use std::collections::HashSet;
use std::env;
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use tracing::info;
use tracing::warn;
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::client::Request;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;
use tungstenite::WebSocket;

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_str(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn next_cid(counter: &mut u64) -> String {
    *counter += 1;
    format!("m{:019}", *counter)
}

fn set_timeout(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    dur: Duration,
) {
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(Some(dur));
        }
        _ => {}
    }
}

fn build_request(addr: &str, user_id: u64) -> Request {
    let mut req = addr.into_client_request().expect("invalid ws url");
    req.headers_mut().insert(
        "x-user-id",
        user_id.to_string().parse().expect("invalid header"),
    );
    req
}

fn drain(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    n: usize,
) {
    for _ in 0..n {
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(data) = serde_json::from_str::<
                    serde_json::Value,
                >(&text)
                {
                    if data["E"].is_array() {
                        warn!("order error: {}", data["E"]);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

fn quote_cycle(
    addr: &str,
    user_id: u64,
    symbol_id: u64,
    mid: i64,
    spread_bps: i64,
    levels: u64,
    qty: i64,
    tick: i64,
    active_cids: &mut HashSet<String>,
    counter: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let req = build_request(addr, user_id);
    let (mut ws, _) = tungstenite::connect(req)?;

    // cancel existing orders, drain 100ms per cancel
    set_timeout(&mut ws, Duration::from_millis(100));
    for cid in active_cids.iter() {
        let msg = serde_json::json!({"C": [cid]}).to_string();
        ws.send(Message::Text(msg.into()))?;
        drain(&mut ws, 1);
    }
    active_cids.clear();

    let spread_offset = (mid * spread_bps / 10000).max(1);
    let step = (spread_offset / 2).max(1);

    set_timeout(&mut ws, Duration::from_millis(200));
    for i in 0..levels as i64 {
        let offset = spread_offset + i * step;

        let bid_cid = next_cid(counter);
        let bid_px = (mid - offset) / tick * tick;
        ws.send(Message::Text(
            serde_json::json!({
                "N": [symbol_id, 0, bid_px, qty, bid_cid, 0]
            })
            .to_string()
            .into(),
        ))?;
        active_cids.insert(bid_cid);

        let ask_cid = next_cid(counter);
        let raw_ask = mid + offset;
        let ask_px = (raw_ask + tick - 1) / tick * tick;
        ws.send(Message::Text(
            serde_json::json!({
                "N": [symbol_id, 1, ask_px, qty, ask_cid, 0]
            })
            .to_string()
            .into(),
        ))?;
        active_cids.insert(ask_cid);

        // drain up to 2 responses per order pair
        drain(&mut ws, 2);
    }

    let _ = ws.close(None);
    Ok(())
}

fn main() {
    install_panic_handler();
    tracing_subscriber::fmt::init();

    let addr = env_str("RSX_GW_WS_ADDR", "ws://localhost:8080");
    let user_id = env_u64("RSX_MAKER_USER_ID", 99);
    let symbol_id = env_u64("RSX_MAKER_SYMBOL", 10);
    let mid = env_u64("RSX_MAKER_MID", 50000) as i64;
    let spread_bps = env_u64("RSX_MAKER_SPREAD", 10) as i64;
    let levels = env_u64("RSX_MAKER_LEVELS", 5);
    let qty = env_u64("RSX_MAKER_QTY", 1_000_000) as i64;
    let tick = env_u64("RSX_MAKER_TICK", 1) as i64;
    let lot = env_u64("RSX_MAKER_LOT", 100_000) as i64;
    let refresh_ms = env_u64("RSX_MAKER_REFRESH", 2000);

    // align qty to lot boundary
    let qty = (qty / lot) * lot;

    info!(
        addr = addr,
        user_id,
        symbol_id,
        mid,
        spread_bps,
        levels,
        qty,
        refresh_ms,
        "rsx-maker started"
    );

    let mut active_cids: HashSet<String> = HashSet::new();
    let mut counter: u64 = 0;

    static RUNNING: AtomicBool = AtomicBool::new(true);
    extern "C" fn on_signal(_: libc::c_int) {
        RUNNING.store(false, Ordering::SeqCst);
    }
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            on_signal as *const () as libc::sighandler_t,
        );
    }

    let mut backoff_s: u64 = 1;

    while RUNNING.load(Ordering::SeqCst) {
        match quote_cycle(
            &addr,
            user_id,
            symbol_id,
            mid,
            spread_bps,
            levels,
            qty,
            tick,
            &mut active_cids,
            &mut counter,
        ) {
            Ok(()) => {
                backoff_s = 1;
                thread::sleep(Duration::from_millis(
                    refresh_ms,
                ));
            }
            Err(e) => {
                warn!(
                    "quote cycle error, reconnect in {}s: {}",
                    backoff_s, e
                );
                active_cids.clear();
                thread::sleep(Duration::from_secs(
                    backoff_s,
                ));
                backoff_s = (backoff_s * 2).min(30);
            }
        }
    }

    info!("rsx-maker stopped");
}
