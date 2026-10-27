use crate::gateway_client::Connection;
use crate::md;
use crate::order;
use crate::order::Gateway;
use crate::order::Sizing;
use crate::output::Failure;
use crate::output::Outcome;
use crate::runtime_utils::block_on;
use crate::stress_report;
use crate::stress_report::Report;
use crate::stress_report::Settings;
use crate::stress_tally::Measure;
use crate::stress_tally::Tally;
use crate::stress_tally::Tracker;
use crate::symbols;
use crate::token::mint_now;
use clap::ValueEnum;
use futures_util::future::join_all;
use rsx_messages::WsFrame;
use rsx_types::client_order::min_order_qty;
use rsx_types::client_order::CidMint;
use serde_json::json;
use serde_json::Value;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use tokio::signal::unix::signal;
use tokio::signal::unix::SignalKind;
use tokio::time::Instant;

#[cfg(test)]
#[path = "stress_test.rs"]
mod stress_test;

/// A symbol id no gateway serves, so `reject` orders fail the gateway's own
/// validation before they reach risk or matching.
pub const REJECT_SYMBOL_ID: u32 = 999_999;

const MAKER_USER_ID: u32 = 99;

const TOKEN_TTL_SECS: u64 = 3600;
const BOOK_WAIT: Duration = Duration::from_secs(2);
const START_DELAY: Duration = Duration::from_millis(200);
const STOP_POLL: Duration = Duration::from_millis(100);
const TIF_GTC: u8 = 0;
const TIF_IOC: u8 = 1;

/// What the run does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Open-loop crossing IOC orders at a target rate for a duration, over
    /// several connections
    Load,
    /// Closed-loop crossing IOC orders, one at a time on one connection
    Probe,
    /// Closed-loop orders for a symbol the gateway does not serve; measures
    /// the gateway's own reject path
    Reject,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Load => "load",
            Mode::Probe => "probe",
            Mode::Reject => "reject",
        }
    }

    fn measure(self) -> Measure {
        match self {
            Mode::Reject => Measure::Rejected,
            Mode::Load | Mode::Probe => Measure::Accepted,
        }
    }

    fn is_crossing(self) -> bool {
        self != Mode::Reject
    }
}

/// How a run is set up; see `run`.
pub struct Options {
    pub gateway: Gateway,
    pub mode: Mode,
    pub symbols: Vec<u32>,
    pub users: u32,
    pub connections: u32,
    pub rate: u32,
    pub duration: Duration,
    pub count: u64,
    pub warmup: u64,
    pub market_url: String,
    pub cross_bps: u32,
    pub sizing: Sizing,
    pub out: Option<PathBuf>,
    pub is_dry_run: bool,
}

/// One symbol's order template: the book's best prices and the smallest
/// quantity that meets the symbol's minimum order size at each of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub symbol_id: u32,
    pub lot_size: i64,
    pub bid_px: i64,
    pub ask_px: i64,
    pub bid_qty: i64,
    pub ask_qty: i64,
    pub buy_px: i64,
    pub sell_px: i64,
    pub buy_qty: i64,
    pub sell_qty: i64,
    pub tif: u8,
}

impl Plan {
    /// The `index`th order of a connection: even ones buy at `buy_px`, odd
    /// ones sell at `sell_px`, so each order crosses and the position nets
    /// out.
    pub fn order(&self, index: u64, cid: String) -> WsFrame {
        let is_buy = index % 2 == 0;
        WsFrame::NewOrder {
            symbol_id: self.symbol_id,
            side: if is_buy { 0 } else { 1 },
            price: if is_buy { self.buy_px } else { self.sell_px },
            qty: if is_buy { self.buy_qty } else { self.sell_qty },
            client_order_id: cid,
            tif: self.tif,
            reduce_only: false,
            post_only: false,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "symbol_id": self.symbol_id,
            "lot_size": self.lot_size,
            "bid_px": self.bid_px,
            "ask_px": self.ask_px,
            "bid_qty": self.bid_qty,
            "ask_qty": self.ask_qty,
            "buy_px": self.buy_px,
            "sell_px": self.sell_px,
            "buy_qty": self.buy_qty,
            "sell_qty": self.sell_qty,
            "buy_lots": self.buy_qty / self.lot_size.max(1),
            "sell_lots": self.sell_qty / self.lot_size.max(1),
        })
    }
}

/// Runs the stress `options` describe and prints the report (one JSON
/// object under `--json`, a summary otherwise); `out` also gets the JSON.
/// With `is_dry_run` it prints the order plan and sends nothing.
///
/// `Load` and `Probe` size every order to its symbol's minimum order size at
/// the book's best price, read from marketdata, so they measure the match
/// path and not the reject path. A run that finds a correctness failure
/// still prints and writes its report, then fails with `Runtime`. An
/// interrupt (SIGINT, SIGTERM) stops sending, lets in-flight orders finish
/// and reports what was measured with `interrupted` true.
pub fn run(options: &Options, is_json: bool) -> Outcome {
    block_on(do_run(options, is_json))
}

async fn do_run(options: &Options, is_json: bool) -> Outcome {
    validate(options)?;
    let plans = make_plans(options).await?;
    if options.is_dry_run {
        let symbols: Vec<Value> = plans.iter().map(Plan::to_json).collect();
        println!(
            "{}",
            json!({ "mode": options.mode.name(), "symbols": symbols })
        );
        return Ok(());
    }
    let (report, is_interrupted) = measure(options, plans).await?;
    if let Some(path) = &options.out {
        write_report(path, &report.json)?;
    }
    if is_json {
        println!("{}", report.json);
    } else {
        print!("{}", report_text(&report.json, is_interrupted));
    }
    if report.failures.is_empty() {
        Ok(())
    } else {
        Err(Failure::runtime(format!(
            "the run is not sound: {}",
            report.failures.join("; ")
        )))
    }
}

fn validate(options: &Options) -> Outcome {
    if options.symbols.is_empty() {
        return Err(Failure::misuse("at least one symbol is required"));
    }
    if options.users == 0 || options.connections == 0 {
        return Err(Failure::misuse("users and connections must be at least 1"));
    }
    match options.mode {
        Mode::Load => {
            if options.rate == 0 || options.duration.is_zero() {
                return Err(Failure::misuse(
                    "load needs a rate and a duration of at least 1",
                ));
            }
            if (options.connections as usize) < options.symbols.len() {
                return Err(Failure::misuse(format!(
                    "{} connections cannot cover {} symbols",
                    options.connections,
                    options.symbols.len()
                )));
            }
        }
        Mode::Probe | Mode::Reject => {
            if options.count == 0 {
                return Err(Failure::misuse(
                    "probe and reject need a count of at least 1",
                ));
            }
        }
    }
    Ok(())
}

async fn make_plans(options: &Options) -> Result<Vec<Plan>, Failure> {
    match options.mode {
        Mode::Reject => Ok(vec![Plan {
            symbol_id: REJECT_SYMBOL_ID,
            lot_size: 1,
            bid_px: 100,
            ask_px: 100,
            bid_qty: 0,
            ask_qty: 0,
            buy_px: 100,
            sell_px: 100,
            buy_qty: 1,
            sell_qty: 1,
            tif: TIF_GTC,
        }]),
        Mode::Probe => plan_symbols(options, &options.symbols[..1]).await,
        Mode::Load => plan_symbols(options, &options.symbols).await,
    }
}

async fn plan_symbols(options: &Options, symbol_ids: &[u32]) -> Result<Vec<Plan>, Failure> {
    let catalog = symbols::fetch(&options.gateway.url).await?;
    let min_notionals = order::fetch_min_notionals(&options.sizing, symbol_ids).await?;
    let mut plans = Vec::with_capacity(symbol_ids.len());
    for symbol_id in symbol_ids {
        let symbol = symbols::find(&catalog, *symbol_id)?;
        let book = md::fetch_snapshot(&options.market_url, *symbol_id, BOOK_WAIT).await?;
        let (Some(bid), Some(ask)) = (book.bids.first(), book.asks.first()) else {
            return Err(Failure::runtime(format!(
                "the book of symbol {symbol_id} has no {} (is the maker running?)",
                if book.bids.is_empty() { "bids" } else { "asks" }
            )));
        };
        let buy_px = cross_up(ask.price, options.cross_bps, symbol.tick_size);
        let sell_px = cross_down(bid.price, options.cross_bps, symbol.tick_size);
        let min_notional = min_notionals
            .get(symbol_id)
            .copied()
            .unwrap_or_else(|| symbol.served_min_notional());
        plans.push(Plan {
            symbol_id: *symbol_id,
            lot_size: symbol.lot_size,
            bid_px: bid.price,
            ask_px: ask.price,
            bid_qty: bid.qty,
            ask_qty: ask.qty,
            buy_px,
            sell_px,
            buy_qty: min_order_qty(min_notional, buy_px, symbol.lot_size),
            sell_qty: min_order_qty(min_notional, sell_px, symbol.lot_size),
            tif: TIF_IOC,
        });
    }
    Ok(plans)
}

fn cross_up(price: i64, bps: u32, tick: i64) -> i64 {
    let tick = i128::from(tick.max(1));
    let raised = i128::from(price) * (10_000 + i128::from(bps));
    let ticks = (raised + 10_000 * tick - 1) / (10_000 * tick);
    i64::try_from(ticks * tick).unwrap_or(i64::MAX)
}

fn cross_down(price: i64, bps: u32, tick: i64) -> i64 {
    let tick = i128::from(tick.max(1));
    let lowered = i128::from(price) * (10_000 - i128::from(bps.min(9_999)));
    let ticks = (lowered / (10_000 * tick)).max(1);
    i64::try_from(ticks * tick).unwrap_or(i64::MAX)
}

struct Run {
    connection: Connection,
    tracker: Tracker,
    plan: Plan,
    cids: Rc<RefCell<CidMint>>,
    stop: Rc<Cell<bool>>,
    timeout: Duration,
    sent: u64,
}

struct Schedule {
    first: Instant,
    interval: Duration,
    end: Instant,
}

async fn measure(options: &Options, plans: Vec<Plan>) -> Result<(Report, bool), Failure> {
    let connection_count = match options.mode {
        Mode::Load => options.connections.min(options.rate),
        Mode::Probe | Mode::Reject => 1,
    };
    let mut interrupt = signal(SignalKind::interrupt())
        .map_err(|error| Failure::runtime(format!("failed to install SIGINT handler: {error}")))?;
    let mut terminate = signal(SignalKind::terminate())
        .map_err(|error| Failure::runtime(format!("failed to install SIGTERM handler: {error}")))?;
    let cids = Rc::new(RefCell::new(CidMint::new()));
    let stop = Rc::new(Cell::new(false));
    let mut runs = open_runs(options, &plans, connection_count, &cids, &stop).await?;
    let first = Instant::now() + START_DELAY;
    let schedules: Vec<Schedule> = (0..runs.len())
        .map(|index| schedule(options, first, index, runs.len()))
        .collect();
    let work = join_all(runs.drain(..).zip(schedules).map(|(run, schedule)| {
        run_connection(options.mode, run, schedule, options.warmup, options.count)
    }));
    tokio::pin!(work);
    let mut is_interrupted = false;
    let tallies = tokio::select! {
        tallies = &mut work => tallies,
        _ = interrupt.recv() => {
            is_interrupted = true;
            stop.set(true);
            work.await
        }
        _ = terminate.recv() => {
            is_interrupted = true;
            stop.set(true);
            work.await
        }
    };
    let elapsed = Instant::now().saturating_duration_since(first);
    let mut per_symbol: BTreeMap<u32, Tally> = BTreeMap::new();
    for (symbol_id, tally) in tallies {
        per_symbol.entry(symbol_id).or_default().merge(&tally);
    }
    let settings = Settings {
        mode: options.mode.name(),
        measure: options.mode.measure(),
        gateway: options.gateway.url.clone(),
        symbols: plans.iter().map(|plan| plan.symbol_id).collect(),
        users: options.users,
        connections: connection_count,
        target_rate: if options.mode == Mode::Load {
            options.rate
        } else {
            0
        },
        duration_s: if options.mode == Mode::Load {
            options.duration.as_secs()
        } else {
            0
        },
        count: if options.mode == Mode::Load {
            0
        } else {
            options.count
        },
        warmup: if options.mode == Mode::Load {
            0
        } else {
            options.warmup
        },
        timeout_ms: options.gateway.timeout.as_millis() as u64,
        is_crossing: options.mode.is_crossing(),
    };
    Ok((
        stress_report::build(&settings, per_symbol, elapsed, is_interrupted),
        is_interrupted,
    ))
}

fn schedule(options: &Options, first: Instant, index: usize, count: usize) -> Schedule {
    let rate = f64::from(options.rate.max(1));
    Schedule {
        first: first + Duration::from_secs_f64(index as f64 / rate),
        interval: Duration::from_secs_f64(count as f64 / rate),
        end: first + options.duration,
    }
}

async fn open_runs(
    options: &Options,
    plans: &[Plan],
    connection_count: u32,
    cids: &Rc<RefCell<CidMint>>,
    stop: &Rc<Cell<bool>>,
) -> Result<Vec<Run>, Failure> {
    let mut openings = Vec::new();
    for index in 0..connection_count {
        let user_id = user_for(index, options.users);
        let minted = mint_now(user_id, TOKEN_TTL_SECS, options.gateway.secret.as_deref())?;
        openings.push(open_run(
            options,
            plans[index as usize % plans.len()].clone(),
            minted.token,
            cids,
            stop,
        ));
    }
    join_all(openings).await.into_iter().collect()
}

async fn open_run(
    options: &Options,
    plan: Plan,
    token: String,
    cids: &Rc<RefCell<CidMint>>,
    stop: &Rc<Cell<bool>>,
) -> Result<Run, Failure> {
    let connection = Connection::open(&options.gateway.url, &token).await?;
    Ok(Run {
        connection,
        tracker: Tracker::new(options.mode.measure()),
        plan,
        cids: cids.clone(),
        stop: stop.clone(),
        timeout: options.gateway.timeout,
        sent: 0,
    })
}

fn user_for(index: u32, users: u32) -> u32 {
    let user_id = index % users + 1;
    if user_id >= MAKER_USER_ID {
        user_id + 1
    } else {
        user_id
    }
}

async fn run_connection(
    mode: Mode,
    mut run: Run,
    schedule: Schedule,
    warmup: u64,
    count: u64,
) -> (u32, Tally) {
    let outcome = match mode {
        Mode::Load => send_on_schedule(&mut run, &schedule).await,
        Mode::Probe | Mode::Reject => send_one_by_one(&mut run, warmup, count).await,
    };
    if let Err(failure) = outcome {
        eprintln!("Error: {}", failure.message());
        run.tracker.tally.count_error("connection_lost");
    }
    run.tracker.finish();
    (run.plan.symbol_id, run.tracker.tally)
}

async fn send_on_schedule(run: &mut Run, schedule: &Schedule) -> Outcome {
    let mut next = schedule.first;
    while next < schedule.end && !run.stop.get() {
        let now = Instant::now();
        if now < next {
            run.read_until(next.min(now + STOP_POLL)).await?;
            continue;
        }
        run.send_order(next).await?;
        next += schedule.interval;
        run.read_until(Instant::now()).await?;
    }
    run.drain().await
}

async fn send_one_by_one(run: &mut Run, warmup: u64, count: u64) -> Outcome {
    for index in 0..warmup + count {
        if run.stop.get() {
            break;
        }
        let sent_at = Instant::now();
        run.send_order(sent_at).await?;
        run.drain().await?;
        run.tracker.finish();
        if index + 1 == warmup {
            run.tracker.tally = Tally::default();
        }
    }
    Ok(())
}

impl Run {
    async fn send_order(&mut self, intended: Instant) -> Outcome {
        let cid = self.cids.borrow_mut().next();
        let frame = self.plan.order(self.sent, cid);
        self.sent += 1;
        match self.connection.send(&frame).await {
            Ok(()) => {
                self.tracker.record_sent(intended, Instant::now());
                Ok(())
            }
            Err(failure) => {
                self.tracker.record_send_failure();
                Err(failure)
            }
        }
    }

    async fn read_one(&mut self, deadline: Instant) -> Result<bool, Failure> {
        let frame = self.connection.next_parsed(deadline).await?;
        let now = Instant::now();
        self.tracker.expire(now, self.timeout);
        match frame {
            Some(frame) => {
                self.tracker.accept(&frame, now);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn read_until(&mut self, deadline: Instant) -> Outcome {
        while self.read_one(deadline).await? {}
        Ok(())
    }

    async fn drain(&mut self) -> Outcome {
        let end = Instant::now() + self.timeout;
        while !self.tracker.is_idle() {
            if !self.read_one(end).await? {
                break;
            }
        }
        Ok(())
    }
}

fn write_report(path: &Path, report: &Value) -> Outcome {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|error| {
            Failure::runtime(format!("cannot create {}: {error}", directory.display()))
        })?;
    }
    fs::write(path, format!("{report}\n"))
        .map_err(|error| Failure::runtime(format!("cannot write {}: {error}", path.display())))
}

fn report_text(report: &Value, is_interrupted: bool) -> String {
    let count = |key: &str| report[key].as_u64().unwrap_or(0);
    let mut text = format!(
        "{} {}: offered {} submitted {} accepted {} rejected {} timeouts {} in {} ms{}\n",
        report["mode"].as_str().unwrap_or(""),
        report["status"].as_str().unwrap_or(""),
        count("offered"),
        count("submitted"),
        count("accepted"),
        count("rejected"),
        count("timeouts"),
        report["elapsed_ms"],
        if is_interrupted { " (interrupted)" } else { "" },
    );
    let latency = &report["latency_us"];
    text.push_str(&format!(
        "latency us: n {} p50 {} p95 {} p99 {} p99.9 {} max {}\n",
        latency["n"],
        latency["p50"],
        latency["p95"],
        latency["p99"],
        latency["p99_9"],
        latency["max"],
    ));
    if let Some(reasons) = report["rejected_by_reason"].as_object() {
        for (reason, count) in reasons {
            text.push_str(&format!("rejected {reason}: {count}\n"));
        }
    }
    if let Some(failures) = report["failures"].as_array() {
        for failure in failures {
            text.push_str(&format!("failure: {}\n", failure.as_str().unwrap_or("")));
        }
    }
    text
}
