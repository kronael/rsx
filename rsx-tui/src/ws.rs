//! WebSocket client transport: `WsConn` behind the `GatewayConn` trait.
//!
//! This is the real client↔gateway leg, speaking webproto-49
//! (specs/2/49-webproto.md) over a `tokio-tungstenite` WebSocket —
//! `QuicConn` (quic.rs) is a transport bench built against a private
//! JSON-over-QUIC shape, not the live gateway's wire format. `WsConn`
//! mirrors `QuicConn`'s structure exactly: `GatewayConn` is
//! synchronous (the UI drains it non-blocking each render tick) but
//! the WS client is async, so `WsConn` owns a background tokio
//! runtime on a dedicated thread and bridges with channels:
//!
//! - `submit` pushes an `OrderReq` onto an unbounded channel; the
//!   async task drains it and writes a `{N:[...]}` text frame.
//! - the async task reads `U`/`F`/`E`/`H` text frames and pushes each
//!   folded `GwEvent` onto a std mpsc channel; `poll_event` drains it
//!   with `try_recv`.
//!
//! One connection, one JWT (minted once at connect time via
//! `mint_jwt`). When `WsConn` drops, the outbound channel closes, the
//! task closes the socket and returns, and the runtime thread exits.
//!
//! Liveness: the read loop echoes gateway heartbeats (reactive), and
//! separately declares the link dead (`Disconnected`) if no inbound
//! frame at all arrives within `DEAD_AFTER` — the echo alone can't
//! tell a live-but-quiet link from a silently-dead socket.
//!
//! Internal casting (rsx-cast) is a separate transport and is
//! untouched.

use crate::conn::GatewayConn;
use crate::conn::GwEvent;
use crate::conn::OrderReq;
use crate::conn::Side;
use crate::conn::Tif;
use futures_util::SinkExt;
use futures_util::StreamExt;
use jsonwebtoken::encode;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::runtime::Builder;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::interval;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Default gateway WS address, used when `RSX_GW_LISTEN` is unset.
pub const DEFAULT_GW_URL: &str = "ws://127.0.0.1:8080";

/// Default JWT signing secret, used when `RSX_GW_JWT_SECRET` is
/// unset. Matches the gateway's dev default (see rsx-gateway auth).
pub const DEFAULT_JWT_SECRET: &str = "rsx-dev-secret-not-for-prod-padpad";

/// How often the read loop wakes to check whether the link has gone
/// silent (see `DEAD_AFTER`).
const LIVENESS_CHECK: Duration = Duration::from_secs(5);

/// Declare the socket dead — emit `Disconnected` — if no inbound frame
/// (not even a gateway heartbeat, which webproto sends every ~10s)
/// arrives within this window. The reactive heartbeat *echo* only
/// proves *we* answered; it says nothing about a socket that silently
/// stopped delivering. This is the missing liveness half: three missed
/// heartbeat windows with zero inbound bytes means the link is dead.
const DEAD_AFTER: Duration = Duration::from_secs(30);

/// Gateway WS address from `RSX_GW_LISTEN`, or `DEFAULT_GW_URL`.
pub fn gateway_url() -> String {
    std::env::var("RSX_GW_LISTEN").unwrap_or_else(|_| DEFAULT_GW_URL.to_owned())
}

/// JWT signing secret from `RSX_GW_JWT_SECRET`, or `DEFAULT_JWT_SECRET`.
pub fn jwt_secret() -> String {
    std::env::var("RSX_GW_JWT_SECRET").unwrap_or_else(|_| DEFAULT_JWT_SECRET.to_owned())
}

/// HS256 claims, matching `rsx-cli/src/bench_probe.rs::Claims` (same
/// field names/shape so the gateway's auth extraction is identical
/// regardless of which client minted the token).
#[derive(Serialize)]
struct Claims<'a> {
    sub: String,
    user_id: u32,
    aud: &'a str,
    iss: &'a str,
    exp: u64,
    jti: String,
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Mint an HS256 JWT for `user_id`, signed with `secret`. Claims shape
/// mirrors `bench_probe::mint_jwt`: `aud` = `rsx-gateway`, `iss` =
/// `rsx-auth`, a fresh `jti` per call (the gateway's JtiTracker
/// rejects a replayed `jti`), `exp` = now + 3600s.
pub fn mint_jwt(user_id: u32, secret: &str) -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let claims = Claims {
        sub: format!("rsx-tui:{user_id}"),
        user_id,
        aud: "rsx-gateway",
        iss: "rsx-auth",
        exp: unix_now_secs() + 3600,
        jti: format!("rsx-tui-{user_id}-{nonce}"),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("INVARIANT: HS256 encode with a non-empty secret never fails")
}

/// A live WebSocket connection to the gateway, drained by the UI each
/// tick.
pub struct WsConn {
    out: UnboundedSender<OrderReq>,
    inbound: Receiver<GwEvent>,
    /// Held so the runtime thread lives as long as the connection. It
    /// exits on its own when `out` drops, so it is never joined.
    _thread: JoinHandle<()>,
}

impl WsConn {
    /// Connect to `url` (e.g. `ws://127.0.0.1:8080`), authenticating
    /// the handshake with `Authorization: Bearer <token>`. `symbol_id`
    /// is the instrument every submitted `OrderReq` trades (the TUI is
    /// single-market; `OrderReq` itself carries no symbol). Returns
    /// immediately; the connection is established on the background
    /// thread and a `GwEvent::Connected` is delivered once the socket
    /// is open.
    pub fn connect(
        url: impl Into<String>,
        token: impl Into<String>,
        symbol_id: u32,
    ) -> io::Result<Self> {
        let url = url.into();
        let token = token.into();
        let (out, out_rx) = unbounded_channel::<OrderReq>();
        let (in_tx, inbound) = std::sync::mpsc::channel::<GwEvent>();
        let thread = std::thread::Builder::new()
            .name("rsx-tui-ws".to_owned())
            .spawn(move || run_thread(url, token, symbol_id, out_rx, in_tx))
            .map_err(io::Error::other)?;
        Ok(WsConn {
            out,
            inbound,
            _thread: thread,
        })
    }

    /// Connect using `RSX_GW_LISTEN`/`RSX_GW_JWT_SECRET` (or their
    /// defaults), minting a fresh JWT for `user_id`.
    pub fn connect_default(user_id: u32, symbol_id: u32) -> io::Result<Self> {
        let token = mint_jwt(user_id, &jwt_secret());
        WsConn::connect(gateway_url(), token, symbol_id)
    }
}

impl GatewayConn for WsConn {
    fn submit(&mut self, order: OrderReq) -> io::Result<()> {
        self.out
            .send(order)
            .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "ws link down"))
    }

    fn poll_event(&mut self) -> Option<GwEvent> {
        match self.inbound.try_recv() {
            Ok(ev) => Some(ev),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }
}

/// Runtime-thread body: build a single-worker tokio runtime and drive
/// the client to completion. A named function per repo convention (no
/// inline `tokio::spawn`).
fn run_thread(
    url: String,
    token: String,
    symbol_id: u32,
    out_rx: UnboundedReceiver<OrderReq>,
    inbound: Sender<GwEvent>,
) {
    let rt = match Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    rt.block_on(run_client(url, token, symbol_id, out_rx, inbound));
}

/// The async client: dial, then pump orders out and events in until
/// either side closes. Pushes `Connected` on handshake success and
/// `Disconnected` on any failure or close.
async fn run_client(
    url: String,
    token: String,
    symbol_id: u32,
    mut out_rx: UnboundedReceiver<OrderReq>,
    inbound: Sender<GwEvent>,
) {
    let mut req = match url.into_client_request() {
        Ok(req) => req,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    let auth_value = match format!("Bearer {token}").parse() {
        Ok(v) => v,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    req.headers_mut().insert("Authorization", auth_value);

    let stream = match connect_async(req).await {
        Ok((stream, _resp)) => stream,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    let (mut write, mut read) = stream.split();
    if inbound.send(GwEvent::Connected).is_err() {
        return;
    }

    // cid is a 20-char zero-padded client order id (specs/2/49
    // §Types). A per-connection monotonic counter is unique for the
    // lifetime of one WsConn, which is all cid dedup needs here.
    let mut cid_counter: u64 = 0;
    // oid -> side for orders this connection submitted, resolved the
    // first time a `U`/`F` frame names the oid (see `fold_order_update`
    // / `claim_pending`). `F` frames carry oid but not side, so this is
    // the only way to recover it without re-plumbing `OrderReq` (out of
    // scope: conn.rs is frozen for this task).
    let mut oid_side: HashMap<String, Side> = HashMap::new();
    // Orders submitted but not yet paired to an oid, oldest first. The
    // `U` accept echoes the order's qty (`filled + remaining`), so a
    // pending is claimed by qty first (`claim_pending`) — robust to a
    // gateway that acks out of submission order (e.g. a fast reject of
    // a later order before an earlier accept) — with FIFO the tiebreak.
    let mut pending: VecDeque<PendingOrder> = VecDeque::new();
    let mut warned_unknown_frame = false;
    // Count of fills whose oid was never paired to a submitted side, so
    // the Buy fallback is surfaced, not silent (see `fold_fill`).
    let mut unknown_side_count: u64 = 0;
    let mut last_inbound = Instant::now();
    let mut liveness = interval(LIVENESS_CHECK);

    loop {
        tokio::select! {
            maybe = out_rx.recv() => match maybe {
                Some(order) => {
                    cid_counter += 1;
                    let cid = format!("{cid_counter:020}");
                    let frame = order_frame(symbol_id, &order, &cid);
                    if write.send(Message::Text(frame)).await.is_err() {
                        let _ = inbound.send(GwEvent::Disconnected);
                        return;
                    }
                    pending.push_back(PendingOrder { qty: order.qty, side: order.side });
                }
                // WsConn dropped: close and exit cleanly.
                None => {
                    let _ = write.close().await;
                    return;
                }
            },
            msg = read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    last_inbound = Instant::now();
                    // Heartbeat: echo the {H:[ts]} frame back so the
                    // gateway does not drop this connection (spec 49-
                    // webproto: client must reply within 10s). Without
                    // this any WsConn idle >10s — e.g. an e2e test
                    // waiting between orders — gets disconnected.
                    if is_heartbeat(&text) {
                        if write.send(Message::Text(text)).await.is_err() {
                            let _ = inbound.send(GwEvent::Disconnected);
                            return;
                        }
                    } else if let Some(ev) = fold_frame(
                        &text,
                        &mut oid_side,
                        &mut pending,
                        &mut warned_unknown_frame,
                        &mut unknown_side_count,
                    ) {
                        if inbound.send(ev).is_err() {
                            return;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    let _ = inbound.send(GwEvent::Disconnected);
                    return;
                }
                // Ping/Pong/Binary/Frame: no webproto content, but still
                // prove the link is alive.
                Some(Ok(_)) => {
                    last_inbound = Instant::now();
                }
                Some(Err(_)) => {
                    let _ = inbound.send(GwEvent::Disconnected);
                    return;
                }
            },
            // Liveness: a socket that silently stops delivering leaks as
            // "alive" under the reactive-echo heartbeat alone. Surface a
            // Disconnected once no inbound frame has arrived for
            // `DEAD_AFTER` (see the const).
            _ = liveness.tick() => {
                if last_inbound.elapsed() > DEAD_AFTER {
                    tracing::warn!(
                        idle_secs = last_inbound.elapsed().as_secs(),
                        "WsConn: no inbound frame within the liveness window, \
                         declaring the link dead",
                    );
                    let _ = inbound.send(GwEvent::Disconnected);
                    return;
                }
            },
        }
    }
}

/// Encode one `OrderReq` as a webproto `{N:[sym,side,px,qty,cid,tif,
/// ro,po]}` text frame (specs/2/49-webproto.md "N: New Order"). `ro`/
/// `po` are always 0: `OrderReq` (conn.rs) has no reduce-only/
/// post-only fields to plumb through.
fn order_frame(symbol_id: u32, order: &OrderReq, cid: &str) -> String {
    let side = match order.side {
        Side::Buy => 0,
        Side::Sell => 1,
    };
    let tif = match order.tif {
        Tif::Gtc => 0,
        Tif::Ioc => 1,
        Tif::Fok => 2,
    };
    serde_json::json!({
        "N": [symbol_id, side, order.price, order.qty, cid, tif, 0, 0],
    })
    .to_string()
}

/// The wire `oid` is a 32-char hex UUIDv7 (order_id_hi ++
/// order_id_lo, see rsx-gateway/src/route.rs `oid_hex`); `GwEvent`'s
/// `oid` fields are `u64` (conn.rs is frozen for this task, so that
/// type cannot change here). Take the low 64 bits (the last 16 hex
/// chars) — this is `order_id_lo` itself, not a hash, so it is exact
/// for any single order; two orders would only collide if their low
/// 64 bits matched, which a UUIDv7 in practice never does.
fn oid_to_u64(hex: &str) -> u64 {
    let start = hex.len().saturating_sub(16);
    u64::from_str_radix(&hex[start..], 16).unwrap_or(0)
}

/// Parse one text frame into its single `{key: [...]}` shape.
fn parse_frame(text: &str) -> Option<(String, Vec<Value>)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    let (key, arr) = obj.iter().next()?;
    Some((key.clone(), arr.as_array()?.clone()))
}

fn value_str(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}

/// True if `text` is a webproto heartbeat frame `{H:[...]}`. The read
/// loop echoes these back to keep the gateway from dropping the link.
fn is_heartbeat(text: &str) -> bool {
    parse_frame(text).map(|(k, _)| k == "H").unwrap_or(false)
}

/// Fold one raw text frame into at most one `GwEvent`. `U` ->
/// `Accepted`/`Done` (see `fold_order_update`), `F` -> `Fill`, `E` ->
/// `Rejected`, `H` -> heartbeat (echoed by the read loop in
/// `run_client` to stay connected; produces no event here). Any other
/// frame type (the public/query channels: `T`/`BBO`/`B`/`D`/`Q`, or
/// unimplemented `O`/`P`/`A`/`FL`/`FN`) has no corresponding `GwEvent`
/// variant in conn.rs on this connection and is dropped, logged once.
fn fold_frame(
    text: &str,
    oid_side: &mut HashMap<String, Side>,
    pending: &mut VecDeque<PendingOrder>,
    warned_unknown_frame: &mut bool,
    unknown_side_count: &mut u64,
) -> Option<GwEvent> {
    let (key, arr) = parse_frame(text)?;
    match key.as_str() {
        "U" => fold_order_update(&arr, oid_side, pending),
        "F" => fold_fill(&arr, oid_side, unknown_side_count),
        "E" => fold_error(&arr),
        "H" => None,
        other => {
            if !*warned_unknown_frame {
                *warned_unknown_frame = true;
                tracing::warn!(
                    frame_type = other,
                    "WsConn: frame type has no GwEvent mapping on this \
                     connection, dropping (logged once)",
                );
            }
            None
        }
    }
}

/// An order submitted on this connection but not yet paired to a
/// gateway-assigned oid. The `U` accept frame echoes the order's qty
/// (`filled + remaining`), so pairing on `qty` recovers the right side
/// even when acks arrive out of submission order; `side` is what a
/// later `F` fill for this oid is labeled with.
struct PendingOrder {
    qty: i64,
    side: Side,
}

/// Claim the pending submitted order whose qty matches `qty` (oldest of
/// an equal-qty group), else — no qty match — the oldest pending of any
/// qty (FIFO). Returns its side, or `None` when nothing is pending.
/// Keying on qty first is what makes side-pairing robust to a gateway
/// that acks out of submission order (the FIFO-only pop mislabeled a
/// Buy as a Sell when a later order was acked first).
fn claim_pending(pending: &mut VecDeque<PendingOrder>, qty: i64) -> Option<Side> {
    let idx = pending.iter().position(|p| p.qty == qty).or({
        if pending.is_empty() {
            None
        } else {
            Some(0)
        }
    })?;
    pending.remove(idx).map(|p| p.side)
}

/// `{U:[oid, status, filled, remaining, reason]}`. `status`: 0 =
/// FILLED, 1 = RESTING, 2 = CANCELLED, 3 = FAILED (specs/2/49
/// "Order Status"; confirmed against rsx-gateway/src/route.rs, which
/// emits status 1 on `OrderInserted`/accept, 0/2 on `OrderDone`/
/// `OrderCancelled`, 3 on `OrderFailed`). The first non-reject `U` for
/// an oid pairs it to a submitted side via `claim_pending` (by qty,
/// FIFO tiebreak) so a later `F` fill can be labeled. A reject (status
/// 3) carries no qty and needs no side, so it must NOT consume a
/// pending — doing so would mis-shift every later pairing.
fn fold_order_update(
    arr: &[Value],
    oid_side: &mut HashMap<String, Side>,
    pending: &mut VecDeque<PendingOrder>,
) -> Option<GwEvent> {
    let oid_hex = arr.first().map(value_str)?;
    let status = arr.get(1).and_then(Value::as_u64)?;
    if status != 3 && !oid_side.contains_key(&oid_hex) {
        let filled = arr.get(2).and_then(Value::as_i64).unwrap_or(0);
        let remaining = arr.get(3).and_then(Value::as_i64).unwrap_or(0);
        if let Some(side) = claim_pending(pending, filled + remaining) {
            oid_side.insert(oid_hex.clone(), side);
        }
    }
    let oid = oid_to_u64(&oid_hex);
    match status {
        1 => Some(GwEvent::Accepted { oid }),
        0 | 2 => Some(GwEvent::Done { oid }),
        3 => {
            let reason = arr.get(4).and_then(Value::as_u64).unwrap_or(0);
            Some(GwEvent::Rejected {
                reason: format!("failure_reason={reason}"),
            })
        }
        _ => None,
    }
}

/// `{F:[taker_oid, maker_oid, px, qty, ts, fee]}`. Side is recovered
/// from `oid_side` (populated by `fold_order_update`) by whichever of
/// `taker_oid`/`maker_oid` this connection has seen — the gateway
/// pushes the same fill frame to both sides of the trade, so exactly
/// one of the two oids belongs to this user unless both legs are this
/// user's own orders (self-trade), in which case `taker_oid` wins.
fn fold_fill(
    arr: &[Value],
    oid_side: &HashMap<String, Side>,
    unknown_side_count: &mut u64,
) -> Option<GwEvent> {
    let taker_oid = arr.first().map(value_str)?;
    let maker_oid = arr.get(1).map(value_str)?;
    let px = arr.get(2).and_then(Value::as_i64)?;
    let qty = arr.get(3).and_then(Value::as_i64)?;

    let (own_oid, side) = match oid_side.get(&taker_oid) {
        Some(side) => (&taker_oid, Some(*side)),
        None => match oid_side.get(&maker_oid) {
            Some(side) => (&maker_oid, Some(*side)),
            None => (&taker_oid, None),
        },
    };
    let side = side.unwrap_or_else(|| {
        *unknown_side_count += 1;
        // Surface the fallback rather than defaulting silently: log on
        // every power-of-two so a persistent mispairing is visible in a
        // busy session without flooding the log.
        if unknown_side_count.is_power_of_two() {
            tracing::warn!(
                unknown_side_count = *unknown_side_count,
                "WsConn: Fill frame's oid was never paired to a submitted \
                 side, defaulting to Buy (count is cumulative)",
            );
        }
        Side::Buy
    });
    Some(GwEvent::Fill {
        oid: oid_to_u64(own_oid),
        px,
        qty,
        side,
    })
}

/// `{E:[code, msg]}`.
fn fold_error(arr: &[Value]) -> Option<GwEvent> {
    let code = arr.first().map(value_str).unwrap_or_default();
    let msg = arr.get(1).map(value_str).unwrap_or_default();
    Some(GwEvent::Rejected {
        reason: format!("{code}: {msg}"),
    })
}

#[cfg(test)]
#[path = "ws_test.rs"]
mod ws_test;
