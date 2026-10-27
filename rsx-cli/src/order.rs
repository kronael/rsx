use crate::gateway_client::Connection;
use crate::outcome::CancelTracker;
use crate::outcome::PlaceTracker;
use crate::outcome::Verdict;
use crate::output::Failure;
use crate::output::Outcome;
use crate::runtime_utils::block_on;
use crate::symbols;
use crate::symbols::Symbol;
use crate::token::mint_now;
use clap::ValueEnum;
use rsx_messages::CancelKey;
use rsx_messages::WsFrame;
use rsx_types::client_order::min_order_qty;
use rsx_types::client_order::CidMint;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;
use tokio::time::Instant;

#[cfg(test)]
#[path = "order_test.rs"]
mod order_test;

const TOKEN_TTL_SECS: u64 = 300;
const CID_MAX_LEN: usize = 20;
const OID_LEN: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tif {
    Gtc,
    Ioc,
    Fok,
}

/// A price as the caller gave it: a decimal in human units, or raw ticks.
#[derive(Debug, PartialEq)]
pub enum PriceInput {
    Human(String),
    Raw(i64),
}

/// A quantity as the caller gave it: a decimal in human units, raw lots, or
/// the smallest quantity that meets the symbol's minimum order size at the
/// order's price.
#[derive(Debug, PartialEq)]
pub enum QtyInput {
    Human(String),
    Raw(i64),
    Minimum,
}

/// One order as asked for, before it is converted to raw units.
#[derive(Debug, PartialEq)]
pub struct OrderInput {
    pub user_id: u32,
    pub symbol_id: u32,
    pub side: Side,
    pub tif: Tif,
    pub price: PriceInput,
    pub qty: QtyInput,
    pub reduce_only: bool,
    pub post_only: bool,
    pub cid: Option<String>,
}

/// How to reach the gateway.
pub struct Gateway {
    pub url: String,
    pub secret: Option<String>,
    pub timeout: Duration,
}

/// Where `--min-size` takes a symbol's minimum notional (raw `price * qty`):
/// `min_notional` if set, else the active risk config in Postgres at
/// `database_url`, else 1 USD from the symbol's decimals.
pub struct Sizing {
    pub min_notional: Option<i64>,
    pub database_url: Option<String>,
}

/// What a cancel names: the client order id or the gateway's order id.
pub enum CancelTarget {
    Cid(String),
    Oid(String),
}

struct ResolvedOrder {
    user_id: u32,
    cid: String,
    frame: WsFrame,
}

/// Sends one new order and prints its outcome; see `Verdict::place_json`. A
/// rejected or timed-out order is a `Runtime` failure after the output.
pub fn place(gateway: &Gateway, input: &OrderInput, sizing: &Sizing, is_json: bool) -> Outcome {
    block_on(do_batch(
        gateway,
        std::slice::from_ref(input),
        sizing,
        is_json,
    ))
}

/// Sends every order of `inputs` in order, one at a time, over one connection
/// per user, and prints one outcome per order as it ends. Orders are
/// resolved and checked before the first is sent. A rejected or timed-out
/// order is counted; the command is a `Runtime` failure after the last one if
/// any was.
pub fn batch(gateway: &Gateway, inputs: &[OrderInput], sizing: &Sizing, is_json: bool) -> Outcome {
    block_on(do_batch(gateway, inputs, sizing, is_json))
}

/// Cancels one pending order as `user_id` and prints the outcome; see
/// `Verdict::cancel_json`. A rejected or timed-out cancel is a `Runtime`
/// failure after the output.
pub fn cancel(gateway: &Gateway, user_id: u32, target: &CancelTarget, is_json: bool) -> Outcome {
    block_on(do_cancel(gateway, user_id, target, is_json))
}

/// Parses `text`, one JSON object per line (blank lines skipped), into
/// orders; a bad line is a `Misuse` failure naming its number. Keys:
/// `user_id`, `symbol_id`, `side` (`buy`|`sell`), `price` (human decimal, a
/// string or number) or `px` (raw), `qty` (human), `qty_raw` or `min_size`
/// (true), and optionally `tif` (`gtc`|`ioc`|`fok`, default `gtc`),
/// `reduce_only`, `post_only` and `cid`.
pub fn parse_batch(text: &str) -> Result<Vec<OrderInput>, Failure> {
    let mut inputs = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let input = parse_batch_line(line)
            .map_err(|error| Failure::misuse(format!("line {}: {error}", index + 1)))?;
        inputs.push(input);
    }
    if inputs.is_empty() {
        return Err(Failure::misuse("no orders in the input"));
    }
    Ok(inputs)
}

/// Reads all of `reader` as batch input.
pub fn read_batch(mut reader: impl Read) -> Result<Vec<OrderInput>, Failure> {
    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .map_err(|error| Failure::runtime(format!("failed to read the batch input: {error}")))?;
    parse_batch(&text)
}

fn decimal_text(raw: &RawValue) -> String {
    let text = raw.get();
    if text.starts_with('"') {
        serde_json::from_str(text).unwrap_or_else(|_| text.to_string())
    } else {
        text.to_string()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchLine {
    user_id: u32,
    symbol_id: u32,
    side: Side,
    #[serde(default = "default_tif")]
    tif: Tif,
    price: Option<Box<RawValue>>,
    px: Option<i64>,
    qty: Option<Box<RawValue>>,
    qty_raw: Option<i64>,
    #[serde(default)]
    min_size: bool,
    #[serde(default)]
    reduce_only: bool,
    #[serde(default)]
    post_only: bool,
    cid: Option<String>,
}

fn default_tif() -> Tif {
    Tif::Gtc
}

fn parse_batch_line(line: &str) -> Result<OrderInput, String> {
    let parsed: BatchLine = serde_json::from_str(line).map_err(|error| error.to_string())?;
    let price = match (parsed.price, parsed.px) {
        (Some(price), None) => PriceInput::Human(decimal_text(&price)),
        (None, Some(px)) => PriceInput::Raw(px),
        _ => return Err("exactly one of price and px is required".to_string()),
    };
    let qty = match (parsed.qty, parsed.qty_raw, parsed.min_size) {
        (Some(qty), None, false) => QtyInput::Human(decimal_text(&qty)),
        (None, Some(qty_raw), false) => QtyInput::Raw(qty_raw),
        (None, None, true) => QtyInput::Minimum,
        _ => return Err("exactly one of qty, qty_raw and min_size is required".to_string()),
    };
    Ok(OrderInput {
        user_id: parsed.user_id,
        symbol_id: parsed.symbol_id,
        side: parsed.side,
        tif: parsed.tif,
        price,
        qty,
        reduce_only: parsed.reduce_only,
        post_only: parsed.post_only,
        cid: parsed.cid,
    })
}

/// A client order id the gateway accepts and spec 61 allows: 1 to 20 bytes of
/// `[0-9A-Za-z_-]`.
pub fn check_cid(cid: &str) -> Result<(), Failure> {
    let is_valid = !cid.is_empty()
        && cid.len() <= CID_MAX_LEN
        && cid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if is_valid {
        Ok(())
    } else {
        Err(Failure::misuse(format!(
            "cid {cid:?} must be 1 to {CID_MAX_LEN} characters of 0-9 A-Z a-z - _"
        )))
    }
}

/// Classifies a cancel key by length as the gateway does: 32 characters is an
/// order id, 1 to 20 a client order id.
pub fn cancel_target(key: &str) -> Result<CancelTarget, Failure> {
    if key.len() == OID_LEN && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(CancelTarget::Oid(key.to_ascii_lowercase()));
    }
    check_cid(key)?;
    Ok(CancelTarget::Cid(key.to_string()))
}

fn needs_catalog(input: &OrderInput) -> bool {
    matches!(input.price, PriceInput::Human(_))
        || matches!(input.qty, QtyInput::Human(_) | QtyInput::Minimum)
}

fn resolve(
    input: &OrderInput,
    symbols: &[Symbol],
    min_notionals: &HashMap<u32, i64>,
    cids: &mut CidMint,
) -> Result<ResolvedOrder, Failure> {
    let symbol = if needs_catalog(input) {
        Some(symbols::find(symbols, input.symbol_id)?)
    } else {
        None
    };
    let price = match (&input.price, symbol) {
        (PriceInput::Raw(price), _) => *price,
        (PriceInput::Human(text), Some(symbol)) => {
            symbols::to_raw(text, symbol.price_decimals).map_err(Failure::misuse)?
        }
        (PriceInput::Human(_), None) => return Err(Failure::misuse("a human price needs symbols")),
    };
    let qty = match (&input.qty, symbol) {
        (QtyInput::Raw(qty), _) => *qty,
        (QtyInput::Human(text), Some(symbol)) => {
            symbols::to_raw(text, symbol.qty_decimals).map_err(Failure::misuse)?
        }
        (QtyInput::Minimum, Some(symbol)) => {
            let min_notional = match min_notionals.get(&input.symbol_id) {
                Some(min_notional) => *min_notional,
                None => symbol.default_min_notional(),
            };
            min_order_qty(min_notional, price, symbol.lot_size)
        }
        (_, None) => return Err(Failure::misuse("a human quantity needs symbols")),
    };
    let cid = match &input.cid {
        Some(cid) => {
            check_cid(cid)?;
            cid.clone()
        }
        None => cids.next(),
    };
    let frame = WsFrame::NewOrder {
        symbol_id: input.symbol_id,
        side: match input.side {
            Side::Buy => 0,
            Side::Sell => 1,
        },
        price,
        qty,
        client_order_id: cid.clone(),
        tif: match input.tif {
            Tif::Gtc => 0,
            Tif::Ioc => 1,
            Tif::Fok => 2,
        },
        reduce_only: input.reduce_only,
        post_only: input.post_only,
    };
    Ok(ResolvedOrder {
        user_id: input.user_id,
        cid,
        frame,
    })
}

async fn do_batch(
    gateway: &Gateway,
    inputs: &[OrderInput],
    sizing: &Sizing,
    is_json: bool,
) -> Outcome {
    let symbols = if inputs.iter().any(needs_catalog) {
        symbols::fetch(&gateway.url).await?
    } else {
        Vec::new()
    };
    let min_notionals = load_min_notionals(sizing, inputs).await?;
    let mut cids = CidMint::new();
    let mut orders = Vec::with_capacity(inputs.len());
    for input in inputs {
        orders.push(resolve(input, &symbols, &min_notionals, &mut cids)?);
    }
    let mut connections: HashMap<u32, Connection> = HashMap::new();
    let mut failed = 0;
    for order in &orders {
        let connection = match connections.entry(order.user_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(open(gateway, order.user_id).await?),
        };
        let started = Instant::now();
        let verdict = place_order(connection, &order.frame, started + gateway.timeout).await?;
        let rtt_us = elapsed_us(started);
        if is_json {
            println!("{}", verdict.place_json(&order.cid, rtt_us));
        } else {
            println!("{}", verdict.to_text(&order.cid, rtt_us));
        }
        if verdict.status.is_failure() {
            failed += 1;
        }
    }
    if failed > 0 {
        return Err(Failure::runtime(format!(
            "{failed} of {} orders were rejected or timed out",
            orders.len()
        )));
    }
    Ok(())
}

async fn load_min_notionals(
    sizing: &Sizing,
    inputs: &[OrderInput],
) -> Result<HashMap<u32, i64>, Failure> {
    let mut symbol_ids = Vec::new();
    for input in inputs {
        if matches!(input.qty, QtyInput::Minimum) {
            symbol_ids.push(input.symbol_id);
        }
    }
    symbol_ids.sort_unstable();
    symbol_ids.dedup();
    fetch_min_notionals(sizing, &symbol_ids).await
}

/// The minimum notional (raw `price * qty`) for each of `symbol_ids` as
/// `sizing` says; a symbol it does not name is absent from the map.
pub async fn fetch_min_notionals(
    sizing: &Sizing,
    symbol_ids: &[u32],
) -> Result<HashMap<u32, i64>, Failure> {
    if let Some(min_notional) = sizing.min_notional {
        return Ok(symbol_ids.iter().map(|id| (*id, min_notional)).collect());
    }
    match &sizing.database_url {
        Some(url) if !symbol_ids.is_empty() => symbols::fetch_min_notionals(url, symbol_ids).await,
        _ => Ok(HashMap::new()),
    }
}

async fn place_order(
    connection: &mut Connection,
    frame: &WsFrame,
    deadline: Instant,
) -> Result<Verdict, Failure> {
    connection.send(frame).await?;
    let mut tracker = PlaceTracker::new();
    loop {
        let Some(received) = connection.next_frame(deadline).await? else {
            return Ok(tracker.expire());
        };
        if let Some(verdict) = tracker.accept(&received.frame, received.raw) {
            return Ok(verdict);
        }
    }
}

async fn do_cancel(
    gateway: &Gateway,
    user_id: u32,
    target: &CancelTarget,
    is_json: bool,
) -> Outcome {
    let mut connection = open(gateway, user_id).await?;
    let started = Instant::now();
    let verdict = cancel_order(&mut connection, target, started + gateway.timeout).await?;
    let rtt_us = elapsed_us(started);
    let (key, cid) = match target {
        CancelTarget::Cid(cid) => (cid.as_str(), Some(cid.as_str())),
        CancelTarget::Oid(oid) => (oid.as_str(), None),
    };
    if is_json {
        println!("{}", verdict.cancel_json(cid, rtt_us));
    } else {
        println!("{}", verdict.to_text(key, rtt_us));
    }
    if verdict.status.is_failure() {
        return Err(Failure::runtime(format!(
            "the cancel of {key} was {}",
            verdict.status.as_str()
        )));
    }
    Ok(())
}

async fn cancel_order(
    connection: &mut Connection,
    target: &CancelTarget,
    deadline: Instant,
) -> Result<Verdict, Failure> {
    let (key, order_id) = match target {
        CancelTarget::Cid(cid) => (CancelKey::ClientOrderId(cid.clone()), None),
        CancelTarget::Oid(oid) => (CancelKey::OrderId(oid.clone()), Some(oid.clone())),
    };
    connection.send(&WsFrame::Cancel { key }).await?;
    let mut tracker = CancelTracker::new(order_id);
    loop {
        let Some(received) = connection.next_frame(deadline).await? else {
            return Ok(tracker.expire());
        };
        if let Some(verdict) = tracker.accept(&received.frame, received.raw) {
            return Ok(verdict);
        }
    }
}

async fn open(gateway: &Gateway, user_id: u32) -> Result<Connection, Failure> {
    let minted = mint_now(user_id, TOKEN_TTL_SECS, gateway.secret.as_deref())?;
    Connection::open(&gateway.url, &minted.token).await
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros() as u64
}
