use crate::output::Failure;
use crate::output::Outcome;
use crate::runtime_utils::block_on;
use futures_util::SinkExt;
use futures_util::StreamExt;
use rsx_marketdata::subscription::CHANNEL_DEPTH;
use rsx_marketdata::types::L2Level;
use rsx_marketdata::types::L2Snapshot;
use rsx_marketdata::wire::decode;
use rsx_marketdata::wire::Frame;
use serde_json::json;
use serde_json::Value;
use std::io::Write;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::net::TcpStream;
use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;
use tokio::time::interval_at;
use tokio::time::timeout;
use tokio::time::Instant;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

#[cfg(test)]
#[path = "md_test.rs"]
mod md_test;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Marketdata drops a subscriber silent for 10s; one heartbeat per 4s keeps a
/// follow connected.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(4);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Prints the L2 book marketdata serves for `symbol_id`: connects, subscribes
/// to depth, waits up to `wait` for the snapshot and prints it. An empty book
/// is a snapshot with empty sides, not a failure. JSON is one object, see
/// `snapshot_json`.
pub fn snapshot(url: &str, symbol_id: u32, wait: Duration, is_json: bool) -> Outcome {
    let snapshot = block_on(do_snapshot(url, symbol_id, wait))?;
    if is_json {
        println!("{}", snapshot_json(&snapshot));
    } else {
        print!("{}", snapshot_text(&snapshot));
    }
    Ok(())
}

/// Streams every frame for `symbols` until `count` frames were printed, a
/// signal arrives, or the connection ends (a `Runtime` failure). JSON is one
/// object per line, see `frame_json`.
pub fn follow(
    url: &str,
    symbols: &[u32],
    channels: u32,
    count: Option<u64>,
    is_json: bool,
) -> Outcome {
    block_on(follow_loop(url, symbols, channels, count, is_json))
}

async fn do_snapshot(url: &str, symbol_id: u32, wait: Duration) -> Result<L2Snapshot, Failure> {
    let mut socket = connect(url).await?;
    subscribe(&mut socket, symbol_id, CHANNEL_DEPTH).await?;
    match timeout(wait, read_snapshot(&mut socket, url, symbol_id)).await {
        Ok(outcome) => outcome,
        Err(_) => Err(Failure::runtime(format!(
            "no snapshot for symbol {symbol_id} from {url} within {} ms",
            wait.as_millis()
        ))),
    }
}

async fn connect(url: &str) -> Result<Socket, Failure> {
    match timeout(CONNECT_TIMEOUT, connect_async(url)).await {
        Ok(Ok((socket, _))) => Ok(socket),
        Ok(Err(error)) => Err(Failure::runtime(format!(
            "failed to connect to {url}: {error}"
        ))),
        Err(_) => Err(Failure::runtime(format!(
            "timed out connecting to {url} after {} s",
            CONNECT_TIMEOUT.as_secs()
        ))),
    }
}

async fn subscribe(socket: &mut Socket, symbol_id: u32, channels: u32) -> Result<(), Failure> {
    let request = json!({ "S": [symbol_id, channels] }).to_string();
    send(socket, request).await
}

async fn send(socket: &mut Socket, text: String) -> Result<(), Failure> {
    socket
        .send(Message::Text(text))
        .await
        .map_err(|error| Failure::runtime(format!("failed to send to marketdata: {error}")))
}

async fn read_snapshot(
    socket: &mut Socket,
    url: &str,
    symbol_id: u32,
) -> Result<L2Snapshot, Failure> {
    loop {
        let Some(frame) = next_frame(socket, url).await? else {
            continue;
        };
        if let Frame::Snapshot(snapshot) = frame {
            if snapshot.symbol_id == symbol_id {
                return Ok(snapshot);
            }
        }
    }
}

/// The next feed frame, `None` for a message that carries none (text, ping,
/// pong). A closed connection or an undecodable frame is a failure.
async fn next_frame(socket: &mut Socket, url: &str) -> Result<Option<Frame>, Failure> {
    match socket.next().await {
        Some(Ok(Message::Binary(bytes))) => decode(&bytes)
            .map(Some)
            .map_err(|error| Failure::runtime(format!("bad frame from {url}: {error}"))),
        Some(Ok(Message::Close(_))) | None => {
            Err(Failure::runtime(format!("connection to {url} closed")))
        }
        Some(Ok(_)) => Ok(None),
        Some(Err(error)) => Err(Failure::runtime(format!("read from {url} failed: {error}"))),
    }
}

async fn follow_loop(
    url: &str,
    symbols: &[u32],
    channels: u32,
    count: Option<u64>,
    is_json: bool,
) -> Outcome {
    let mut interrupt = signal(SignalKind::interrupt())
        .map_err(|error| Failure::runtime(format!("failed to install SIGINT handler: {error}")))?;
    let mut terminate = signal(SignalKind::terminate())
        .map_err(|error| Failure::runtime(format!("failed to install SIGTERM handler: {error}")))?;
    let mut socket = connect(url).await?;
    for symbol_id in symbols {
        subscribe(&mut socket, *symbol_id, channels).await?;
    }
    let mut ticker = interval_at(Instant::now() + HEARTBEAT_EVERY, HEARTBEAT_EVERY);
    let mut out = std::io::stdout().lock();
    let mut printed = 0u64;
    while count.is_none_or(|limit| printed < limit) {
        tokio::select! {
            _ = interrupt.recv() => return Ok(()),
            _ = terminate.recv() => return Ok(()),
            _ = ticker.tick() => send(&mut socket, heartbeat_request()).await?,
            frame = next_frame(&mut socket, url) => {
                let Some(frame) = frame? else {
                    continue;
                };
                let line = if is_json {
                    frame_json(&frame).to_string()
                } else {
                    frame_text(&frame)
                };
                writeln!(out, "{line}").map_err(|error| {
                    Failure::runtime(format!("failed to write stdout: {error}"))
                })?;
                printed += 1;
            }
        }
    }
    Ok(())
}

fn heartbeat_request() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    json!({ "H": [now as u64] }).to_string()
}

fn levels_json(levels: &[L2Level], is_counted: bool) -> Value {
    levels
        .iter()
        .map(|level| {
            if is_counted {
                json!([level.price, level.qty, level.count])
            } else {
                json!([level.price, level.qty])
            }
        })
        .collect()
}

/// The `md snapshot` object: `symbol_id`, `bids` and `asks` as `[px, qty]`
/// best first, and `bbo` (`bid_px`, `bid_qty`, `ask_px`, `ask_qty`, `seq`,
/// `ts_ns`) taken from the best level of each side, 0 for an empty side, null
/// when both sides are empty.
pub fn snapshot_json(snapshot: &L2Snapshot) -> Value {
    let bbo = if snapshot.bids.is_empty() && snapshot.asks.is_empty() {
        Value::Null
    } else {
        let (bid_px, bid_qty) = best(&snapshot.bids);
        let (ask_px, ask_qty) = best(&snapshot.asks);
        json!({
            "bid_px": bid_px,
            "bid_qty": bid_qty,
            "ask_px": ask_px,
            "ask_qty": ask_qty,
            "seq": snapshot.seq,
            "ts_ns": snapshot.timestamp_ns,
        })
    };
    json!({
        "symbol_id": snapshot.symbol_id,
        "bbo": bbo,
        "bids": levels_json(&snapshot.bids, false),
        "asks": levels_json(&snapshot.asks, false),
    })
}

fn best(levels: &[L2Level]) -> (i64, i64) {
    match levels.first() {
        Some(level) => (level.price, level.qty),
        None => (0, 0),
    }
}

/// The `md follow` object for one frame; `kind` is `bbo`, `l2_snapshot`,
/// `l2_delta`, `trade` or `heartbeat`. Prices and quantities are raw i64;
/// `side` is 0=bid 1=ask on a delta, `taker_side` 0=buy 1=sell; a delta with
/// `qty` 0 removes the level; an empty bbo side has `px` 0.
pub fn frame_json(frame: &Frame) -> Value {
    match frame {
        Frame::Bbo(bbo) => json!({
            "kind": "bbo",
            "symbol_id": bbo.symbol_id,
            "bid_px": bbo.bid_px,
            "bid_qty": bbo.bid_qty,
            "bid_count": bbo.bid_count,
            "ask_px": bbo.ask_px,
            "ask_qty": bbo.ask_qty,
            "ask_count": bbo.ask_count,
            "ts_ns": bbo.timestamp_ns,
            "seq": bbo.seq,
        }),
        Frame::Snapshot(snapshot) => json!({
            "kind": "l2_snapshot",
            "symbol_id": snapshot.symbol_id,
            "bids": levels_json(&snapshot.bids, true),
            "asks": levels_json(&snapshot.asks, true),
            "ts_ns": snapshot.timestamp_ns,
            "seq": snapshot.seq,
        }),
        Frame::Delta(delta) => json!({
            "kind": "l2_delta",
            "symbol_id": delta.symbol_id,
            "side": delta.side,
            "px": delta.price,
            "qty": delta.qty,
            "count": delta.count,
            "ts_ns": delta.timestamp_ns,
            "seq": delta.seq,
        }),
        Frame::Trade(trade) => json!({
            "kind": "trade",
            "symbol_id": trade.symbol_id,
            "px": trade.price,
            "qty": trade.qty,
            "taker_side": trade.taker_side,
            "ts_ns": trade.timestamp_ns,
            "seq": trade.seq,
        }),
        Frame::Heartbeat { timestamp_ms } => json!({
            "kind": "heartbeat",
            "ts_ms": timestamp_ms,
        }),
    }
}

fn side_name(side: u8, names: [&'static str; 2]) -> &'static str {
    if side == 0 {
        names[0]
    } else {
        names[1]
    }
}

pub(crate) fn frame_text(frame: &Frame) -> String {
    match frame {
        Frame::Bbo(bbo) => format!(
            "bbo sym={} bid={}x{} ask={}x{} seq={}",
            bbo.symbol_id, bbo.bid_px, bbo.bid_qty, bbo.ask_px, bbo.ask_qty, bbo.seq
        ),
        Frame::Snapshot(snapshot) => format!(
            "l2_snapshot sym={} bids={} asks={} seq={}",
            snapshot.symbol_id,
            snapshot.bids.len(),
            snapshot.asks.len(),
            snapshot.seq
        ),
        Frame::Delta(delta) => format!(
            "l2_delta sym={} {} {}x{} seq={}",
            delta.symbol_id,
            side_name(delta.side, ["bid", "ask"]),
            delta.price,
            delta.qty,
            delta.seq
        ),
        Frame::Trade(trade) => format!(
            "trade sym={} px={} qty={} taker={} seq={}",
            trade.symbol_id,
            trade.price,
            trade.qty,
            side_name(trade.taker_side, ["buy", "sell"]),
            trade.seq
        ),
        Frame::Heartbeat { timestamp_ms } => format!("heartbeat ts_ms={timestamp_ms}"),
    }
}

pub(crate) fn snapshot_text(snapshot: &L2Snapshot) -> String {
    let mut text = format!("symbol {} seq {}\n", snapshot.symbol_id, snapshot.seq);
    for level in snapshot.asks.iter().rev() {
        text.push_str(&format!("ask {} x {}\n", level.price, level.qty));
    }
    for level in &snapshot.bids {
        text.push_str(&format!("bid {} x {}\n", level.price, level.qty));
    }
    text
}
