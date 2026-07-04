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
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::runtime::Builder;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Default gateway WS address, used when `RSX_GW_LISTEN` is unset.
pub const DEFAULT_GW_URL: &str = "ws://127.0.0.1:8080";

/// Default JWT signing secret, used when `RSX_GW_JWT_SECRET` is
/// unset. Matches the gateway's dev default (see rsx-gateway auth).
pub const DEFAULT_JWT_SECRET: &str = "rsx-dev-secret-not-for-prod-padpad";

/// Gateway WS address from `RSX_GW_LISTEN`, or `DEFAULT_GW_URL`.
pub fn gateway_url() -> String {
    std::env::var("RSX_GW_LISTEN").unwrap_or_else(|_| DEFAULT_GW_URL.to_owned())
}

/// JWT signing secret from `RSX_GW_JWT_SECRET`, or `DEFAULT_JWT_SECRET`.
pub fn jwt_secret() -> String {
    std::env::var("RSX_GW_JWT_SECRET")
        .unwrap_or_else(|_| DEFAULT_JWT_SECRET.to_owned())
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
        Ok(WsConn { out, inbound, _thread: thread })
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
        self.out.send(order).map_err(|_| {
            io::Error::new(io::ErrorKind::NotConnected, "ws link down")
        })
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
    // first time a `U` frame names the oid (see `fold_order_update`).
    // `F` frames carry oid but not side, so this is the only way to
    // recover it without re-plumbing `OrderReq` (out of scope: conn.rs
    // is frozen for this task).
    let mut oid_side: HashMap<String, Side> = HashMap::new();
    // Sides of orders submitted but not yet paired to an oid, oldest
    // first. Gateway acks are FIFO per connection, so the first `U`
    // seen after a submit pairs with that submit's side.
    let mut pending_side: VecDeque<Side> = VecDeque::new();
    let mut warned_unknown_frame = false;
    let mut warned_unknown_side = false;

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
                    pending_side.push_back(order.side);
                }
                // WsConn dropped: close and exit cleanly.
                None => {
                    let _ = write.close().await;
                    return;
                }
            },
            msg = read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
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
                    } else {
                        let events = fold_frame(
                            &text,
                            &mut oid_side,
                            &mut pending_side,
                            &mut warned_unknown_frame,
                            &mut warned_unknown_side,
                        );
                        for ev in events {
                            if inbound.send(ev).is_err() {
                                return;
                            }
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    let _ = inbound.send(GwEvent::Disconnected);
                    return;
                }
                // Ping/Pong/Binary/Frame: no webproto content, ignore.
                Some(Ok(_)) => {}
                Some(Err(_)) => {
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
    v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())
}

/// True if `text` is a webproto heartbeat frame `{H:[...]}`. The read
/// loop echoes these back to keep the gateway from dropping the link.
fn is_heartbeat(text: &str) -> bool {
    parse_frame(text).map(|(k, _)| k == "H").unwrap_or(false)
}

/// Fold one raw text frame into zero or more `GwEvent`s. `U` ->
/// `Accepted`/`Done` (see `fold_order_update`), `F` -> `Fill`, `E` ->
/// `Rejected`, `H` -> heartbeat (echoed by the read loop in
/// `run_client` to stay connected; produces no event here). Any other frame type (the
/// public/query channels: `T`/`BBO`/`B`/`D`/`Q`, or unimplemented
/// `O`/`P`/`A`/`FL`/`FN`) has no corresponding `GwEvent` variant in
/// conn.rs on this connection and is dropped, logged once.
fn fold_frame(
    text: &str,
    oid_side: &mut HashMap<String, Side>,
    pending_side: &mut VecDeque<Side>,
    warned_unknown_frame: &mut bool,
    warned_unknown_side: &mut bool,
) -> Vec<GwEvent> {
    let Some((key, arr)) = parse_frame(text) else {
        return Vec::new();
    };
    match key.as_str() {
        "U" => fold_order_update(&arr, oid_side, pending_side)
            .into_iter()
            .collect(),
        "F" => fold_fill(&arr, oid_side, warned_unknown_side)
            .into_iter()
            .collect(),
        "E" => fold_error(&arr).into_iter().collect(),
        "H" => Vec::new(),
        other => {
            if !*warned_unknown_frame {
                *warned_unknown_frame = true;
                tracing::warn!(
                    frame_type = other,
                    "WsConn: frame type has no GwEvent mapping on this \
                     connection, dropping (logged once)",
                );
            }
            Vec::new()
        }
    }
}

/// `{U:[oid, status, filled, remaining, reason]}`. `status`: 0 =
/// FILLED, 1 = RESTING, 2 = CANCELLED, 3 = FAILED (specs/2/49
/// "Order Status"; confirmed against rsx-gateway/src/route.rs, which
/// emits status 1 on `OrderInserted`/accept, 0/2 on `OrderDone`/
/// `OrderCancelled`, 3 on `OrderFailed`). The first `U` seen for an
/// oid claims the oldest pending submitted side (FIFO — gateway acks
/// orders on one connection in submission order).
fn fold_order_update(
    arr: &[Value],
    oid_side: &mut HashMap<String, Side>,
    pending_side: &mut VecDeque<Side>,
) -> Option<GwEvent> {
    let oid_hex = arr.first().map(value_str)?;
    let status = arr.get(1).and_then(Value::as_u64)?;
    oid_side
        .entry(oid_hex.clone())
        .or_insert_with(|| pending_side.pop_front().unwrap_or(Side::Buy));
    let oid = oid_to_u64(&oid_hex);
    match status {
        1 => Some(GwEvent::Accepted { oid }),
        0 | 2 => Some(GwEvent::Done { oid }),
        3 => {
            let reason = arr.get(4).and_then(Value::as_u64).unwrap_or(0);
            Some(GwEvent::Rejected { reason: format!("failure_reason={reason}") })
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
    warned_unknown_side: &mut bool,
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
        if !*warned_unknown_side {
            *warned_unknown_side = true;
            tracing::warn!(
                "WsConn: Fill frame's oid was never seen in a U frame, \
                 defaulting side to Buy (logged once)",
            );
        }
        Side::Buy
    });
    Some(GwEvent::Fill { oid: oid_to_u64(own_oid), px, qty, side })
}

/// `{E:[code, msg]}`.
fn fold_error(arr: &[Value]) -> Option<GwEvent> {
    let code = arr.first().map(value_str).unwrap_or_default();
    let msg = arr.get(1).map(value_str).unwrap_or_default();
    Some(GwEvent::Rejected { reason: format!("{code}: {msg}") })
}
