//! Risk-engine FLOOD bench: behaviour as offered load grows past capacity.
//! ======================================================================
//!
//! THE QUESTION
//! ------------
//! `risk_throughput_bench` gives the engine's service CEILING. This bench asks
//! the other half: as OFFERED order rate climbs from well below capacity to
//! past saturation, HOW does the shard degrade? Does latency stay flat then
//! KNEE upward at capacity? Does backpressure engage? Does anything stall/drop?
//! The point is to SEE the curve, not to report one number.
//!
//! OPEN-LOOP, SINGLE-THREAD, SCHEDULE-DRIVEN (no coordinated omission)
//! ------------------------------------------------------------------
//! The risk shard in production is single-threaded; the recv loop calls
//! `process_order` directly off the net buffer. We model OFFERED LOAD as a fixed
//! arrival SCHEDULE: order i is "due" at `start + i*gap`, gap = 1e9/rate. We do
//! NOT run a separate generator thread feeding a ring -- the oracle design
//! critique (#5/#7/#8) flagged that an input ring would make the HARNESS QUEUE
//! the system-under-test and let generator jitter masquerade as engine latency.
//! Instead the engine thread itself walks the schedule:
//!
//!   for i in 0..N:
//!       due = start + i*gap
//!       if now < due { spin until due }          // arrival pacing
//!       process_order(prebuilt[i])               // the real hot path
//!       latency[i] = now_after - due             // SCHEDULED latency
//!
//! `latency = completion - DUE` (not completion - start-of-call). Once the
//! engine falls behind the schedule, `now` is already past `due` for the next
//! order, so it processes back-to-back and the lateness ACCUMULATES -- exactly
//! the coordinated-omission-free signal we want. Below capacity the engine waits
//! for each due time and latency ~= one service time; at/above capacity it can
//! never catch up and scheduled latency grows without bound within the window.
//! This is the KNEE. (Oracle design critique #2, #7, #9.)
//!
//! All inputs are pre-built (no alloc in the loop). The "queue depth" is the
//! schedule backlog: `backlog = orders_due_by_now - orders_done`.
//!
//! BACKPRESSURE: PERSIST DRAIN RATE IS A SWEPT PARAMETER, NOT A CONSTANT
//! --------------------------------------------------------------------
//! The real backpressure signal is the persist ring filling: `process_fill`
//! pushes ~6 events onto the persist SPSC ring; if `push` fails the shard sets
//! `backpressured = true`, and the real `tick()` stalls the hot path until the
//! sidecar drains. There is no single "correct" PG drain rate, so we do NOT bake
//! one in (oracle design critique #11/#12). The order flood (Table A) is pure CPU
//! saturation (`process_order` does not push persist on the accept path; the
//! durable freeze is written by `confirm_freeze` on ME ack, off this path). A
//! SECOND pass (Table B, "fill-flood") drives `process_fill` with the persist
//! drain throttled to a chosen events/s. To measure the STALL faithfully (not the
//! event-drop inversion), the engine self-accounts ring occupancy
//! (events pushed - events drained) and gates with hysteresis: it stalls at the
//! HIGH watermark and resumes only once the throttled drain pulls occupancy back
//! to the LOW watermark -- i.e. it never overflows + drops, it STALLS, exactly
//! like the real loop. We sweep drain rate: unthrottled / fast / matched / slow,
//! and report backpressure ONSET, total stall time, and ring high-water.
//!
//! HOW TO READ
//! -----------
//! Table A (order flood): per offered rate -> achieved/s, achieved/offered %,
//!   p50/p99/p999/max scheduled latency (us), max schedule backlog, KNEE marker.
//!   Knee = first rate where achieved/offered < 0.95 (the engine can no longer
//!   keep the offered schedule). p99 is a column, NOT a knee criterion.
//! Table B (fill flood + persist backpressure): per (drain rate) -> achieved/s,
//!   p99 service latency, whether backpressure engaged, time-to-first-backpressure,
//!   total backpressured (stall) time, persist ring high-water occupancy.
//!
//! ENV KNOBS
//!   RFB_RATES="..."   offered req/s sweep (default geometric around capacity)
//!   RFB_ORDERS=2000000 orders per rate (window = ORDERS/rate, capped by time)
//!   RFB_MAX_MS=4000    hard time cap per rate (so a saturated rate still ends)
//!   RFB_USERS=10000    seeded accounts
//!   RFB_SYMBOLS=16     symbols
//!   RFB_HOT_USERS=64   active trading users
//!   RFB_DEPTH=8        resting orders per hot user (frozen_for_user work)
//!   RFB_WARMUP=50000   un-recorded warm-up orders before each rate
//!   RFB_FILL_RATES="..." persist-drain events/s sweep for the fill flood
//!   RFB_SKIP_FILL=0    set 1 to skip table B
//!
//! CAVEATS
//!   * Engine-only (no UDP/WS/decode). Single shard, single core. Per-shard.
//!   * The schedule pacing uses a busy-spin on `Instant::now()`; at very high
//!     rates (gap < ~50ns) the pacing loop itself costs a few ns -- we report
//!     achieved/s so you can see when pacing, not the engine, is the limiter.
//!   * "backlog" is schedule lateness in units of orders, computed at sample
//!     points; it is the open-loop queue depth, not a real ring occupancy.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use hdrhistogram::Histogram;
use rsx_risk::Account;
use rsx_risk::FundingConfig;
use rsx_risk::LiquidationConfig;
use rsx_risk::OrderRequest;
use rsx_risk::ReplicationConfig;
use rsx_risk::RiskShard;
use rsx_risk::ShardConfig;
use rsx_risk::SymbolRiskParams;
use rsx_risk::persist::PersistEvent;
use rsx_risk::types::FillEvent;

const COLLATERAL: i64 = 1_000_000_000_000;
const MARK: i64 = 50_000;
const QTY: i64 = 10;

struct Cfg {
    rates: Vec<u64>,
    orders: u64,
    max_ms: u64,
    users: u32,
    symbols: usize,
    hot_users: u32,
    depth: usize,
    warmup: u64,
    fill_rates: Vec<u64>,
    skip_fill: bool,
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(default)
}

fn env_rates(key: &str, default: Vec<u64>) -> Vec<u64> {
    std::env::var(key)
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect::<Vec<u64>>())
        .filter(|v| !v.is_empty())
        .unwrap_or(default)
}

fn load_cfg() -> Cfg {
    Cfg {
        rates: env_rates(
            "RFB_RATES",
            vec![
                100_000, 250_000, 500_000, 1_000_000, 2_000_000, 3_000_000, 4_000_000, 6_000_000,
                8_000_000, 12_000_000,
            ],
        ),
        orders: env_u64("RFB_ORDERS", 2_000_000),
        max_ms: env_u64("RFB_MAX_MS", 4_000),
        users: env_u64("RFB_USERS", 10_000) as u32,
        symbols: env_u64("RFB_SYMBOLS", 16) as usize,
        hot_users: env_u64("RFB_HOT_USERS", 64) as u32,
        depth: env_u64("RFB_DEPTH", 8) as usize,
        warmup: env_u64("RFB_WARMUP", 50_000),
        fill_rates: env_rates(
            "RFB_FILL_RATES",
            // events/s the persist sidecar can drain. process_fill pushes ~6
            // events/fill, so e.g. 6,000,000 ev/s ~= 1,000,000 fills/s drain.
            // u64::MAX = unthrottled (reveals the cross-core SPSC handoff ceiling).
            vec![u64::MAX, 20_000_000, 5_000_000, 1_000_000],
        ),
        skip_fill: env_u64("RFB_SKIP_FILL", 0) != 0,
    }
}

// ---------------------------------------------------------------------------
// Shard construction + seeding (shared shape with throughput bench)
// ---------------------------------------------------------------------------

fn make_shard(symbols: usize) -> RiskShard {
    let mut params = Vec::with_capacity(symbols);
    let mut taker = Vec::with_capacity(symbols);
    let mut maker = Vec::with_capacity(symbols);
    for _ in 0..symbols {
        params.push(SymbolRiskParams {
            initial_margin_rate: 1000,
            maintenance_margin_rate: 500,
            max_leverage: 10,
        });
        taker.push(5i64);
        maker.push(-1i64);
    }
    let mut shard = RiskShard::new(ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols: symbols,
        symbol_params: params,
        taker_fee_bps: taker,
        maker_fee_bps: maker,
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    });
    for sid in 0..symbols {
        shard.mark_prices[sid] = MARK;
    }
    shard
}

fn order_template(user_id: u32, symbol_id: u32, oid: u64) -> OrderRequest {
    OrderRequest {
        seq: 0,
        user_id,
        symbol_id,
        price: MARK,
        qty: QTY,
        order_id_hi: 0,
        order_id_lo: oid,
        timestamp_ns: 0,
        side: (oid & 1) as u8,
        tif: 0,
        reduce_only: false,
        post_only: false,
        is_liquidation: false,
        _pad: [0; 3],
    }
}

#[inline(always)]
fn now_ns(base: Instant) -> u64 {
    base.elapsed().as_nanos() as u64
}

fn new_hist() -> Histogram<u64> {
    Histogram::<u64>::new_with_bounds(1, 600_000_000, 3).unwrap()
}

// ---------------------------------------------------------------------------
// ORDER FLOOD
// ---------------------------------------------------------------------------

struct FloodResult {
    offered: u64,
    achieved: f64,
    hist: Histogram<u64>,
    max_backlog: u64,
    completed: u64,
}

/// Pre-build the order stream once; reused across rates (orders are released on
/// a sliding window so the frozen map stays bounded, identical content each
/// rate). `prebuilt[i]` is the order due at slot i.
fn build_orders(cfg: &Cfg, n: usize) -> Vec<OrderRequest> {
    let hot = cfg.hot_users.max(1);
    let symbols = cfg.symbols as u32;
    let base: u64 = 1_000_000_000;
    (0..n)
        .map(|i| {
            let u = (i as u64 % hot as u64) as u32;
            let sid = (i as u64 % symbols as u64) as u32;
            order_template(u, sid, base + i as u64)
        })
        .collect()
}

fn fresh_seeded_shard(cfg: &Cfg) -> RiskShard {
    let mut shard = make_shard(cfg.symbols);
    for uid in 0..cfg.users {
        shard.accounts.insert(uid, Account::new(uid, COLLATERAL));
    }
    // resting orders on hot users so frozen_for_user has real work
    let mut oid: u64 = 1;
    for u in 0..cfg.hot_users {
        for d in 0..cfg.depth {
            let sid = (d % cfg.symbols) as u32;
            let _ = shard.process_order(&order_template(u, sid, oid));
            oid += 1;
        }
    }
    shard
}

fn order_flood_at_rate(cfg: &Cfg, orders: &[OrderRequest], rate: u64) -> FloodResult {
    let mut shard = fresh_seeded_shard(cfg);
    let hot = cfg.hot_users.max(1);
    const SLIDE: u64 = 1024;
    let base: u64 = 1_000_000_000;

    // warm-up (unrecorded, back-to-back)
    let warm = cfg.warmup.min(orders.len() as u64);
    for i in 0..warm {
        let o = &orders[i as usize];
        let _ = shard.process_order(o);
        if i >= SLIDE {
            let old = base + (i - SLIDE);
            let ou = ((i - SLIDE) % hot as u64) as u32;
            shard.release_frozen_for_order(ou, 0, old);
        }
    }

    let gap_ns: u64 = (1_000_000_000 / rate.max(1)).max(1);
    let n = orders.len() as u64;
    let total = cfg.orders.min(n);
    let mut hist = new_hist();
    let mut max_backlog: u64 = 0;
    let base_t = Instant::now();
    let start = now_ns(base_t);
    let max_ns = cfg.max_ms * 1_000_000;
    let mut done: u64 = 0;
    let mut i: u64 = warm; // continue oid past warm-up to keep ids unique

    while done < total {
        let slot = done;
        let due = start + slot * gap_ns;
        // arrival pacing: wait until this order is due (open-loop schedule).
        loop {
            let now = now_ns(base_t);
            if now >= due {
                break;
            }
            std::hint::spin_loop();
        }
        // process the real hot path
        let idx = (i % n) as usize;
        // keep the order id monotone + matched to the release below
        let mut o = orders[idx];
        let oid = base + i;
        o.order_id_lo = oid;
        let _ = shard.process_order(&o);
        if i >= SLIDE {
            let old = base + (i - SLIDE);
            let ou = ((i - SLIDE) % hot as u64) as u32;
            shard.release_frozen_for_order(ou, 0, old);
        }
        let after = now_ns(base_t);
        let lat = after.saturating_sub(due).max(1);
        hist.record(lat).ok();
        // schedule backlog = orders that were due by `after` but not yet done.
        let due_by_now = ((after.saturating_sub(start)) / gap_ns).min(total);
        let backlog = due_by_now.saturating_sub(done + 1);
        if backlog > max_backlog {
            max_backlog = backlog;
        }
        done += 1;
        i += 1;
        if after.saturating_sub(start) >= max_ns {
            break;
        }
    }
    let elapsed = now_ns(base_t).saturating_sub(start);
    let achieved = done as f64 / (elapsed as f64 / 1e9);
    FloodResult {
        offered: rate,
        achieved,
        hist,
        max_backlog,
        completed: done,
    }
}

// ---------------------------------------------------------------------------
// FILL FLOOD + persist backpressure (drain rate swept)
// ---------------------------------------------------------------------------

struct FillFloodResult {
    achieved: f64,
    p99_ns: u64,
    backpressure_engaged: bool,
    time_to_first_bp_ms: f64,
    bp_total_ms: f64,
    persist_hwm: usize,
    completed: u64,
}

/// Drive process_fill flat-out (back-to-back, no arrival schedule -- we want to
/// saturate the persist ring), with the persist drain throttled to `drain_rate`
/// events/s. Watch `is_backpressured()` transitions.
fn fill_flood(cfg: &Cfg, drain_rate: u64, fills: u64) -> FillFloodResult {
    let mut shard = make_shard(cfg.symbols);
    for uid in 0..cfg.users {
        shard.accounts.insert(uid, Account::new(uid, COLLATERAL));
    }
    let cap = 1usize << 14; // 16384-slot persist ring
    let (prod, mut cons) = rtrb::RingBuffer::<PersistEvent>::new(cap);
    shard.set_persist_producer(prod);

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let hwm = Arc::new(AtomicU64::new(0));
    // Self-accounted ring occupancy. We do NOT rely on rtrb's `Consumer::slots()`
    // for the gate: under concurrent pop it can under-report (head/tail caching),
    // so occupancy read low even when the ring is full and the engine never
    // stalls -- events then drop and achieved/s perversely stops tracking the
    // drain rate (the inversion, oracle finding #1). Instead the engine counts
    // events it PUSHES (PER_FILL each) and the drain counts events it POPS; the
    // live occupancy is `pushed - drained`, fully under our control.
    let drained_ct = Arc::new(AtomicU64::new(0));
    let drained2 = drained_ct.clone();
    // The drain thread is intentionally NOT pinned: pinning it to a fixed core
    // (esp. core 0, which fields most IRQs/softirqs on a typical box) measured
    // far slower cross-core SPSC pop throughput than letting the scheduler place
    // it. The engine/main thread stays pinned (steady service-time tail); the
    // drain floats.
    let drain = std::thread::Builder::new()
        .name("rfb-persist-drain".into())
        .spawn(move || {
            let infinite = drain_rate == u64::MAX;
            // Token bucket: tokens accrue at `drain_rate` per real second, capped
            // at `bucket_max` (~1ms worth) so the sidecar drains at a STEADY rate
            // and cannot burst-empty a backlog the instant tokens accrue. A
            // cumulative-budget design (the earlier code) let the drain catch up
            // in one burst, so the producer never saw sustained backpressure and
            // achieved/s did not fall with drain rate. The token bucket fixes that.
            let bucket_max = (drain_rate / 1000).max(64) as f64;
            let mut tokens = 0.0f64;
            let mut last = Instant::now();
            let mut drained: u64 = 0;
            loop {
                if stop2.load(Ordering::Relaxed) {
                    while cons.pop().is_ok() {}
                    break;
                }
                if infinite {
                    let mut popped = 0u64;
                    // publish progress frequently so the engine's pushed-drained
                    // occupancy estimate stays fresh (a batch-end-only store lets
                    // the fast engine lap the publish cadence and falsely stall).
                    while cons.pop().is_ok() {
                        popped += 1;
                        if popped & 0x3F == 0 {
                            drained2.store(drained + popped, Ordering::Relaxed);
                        }
                    }
                    if popped > 0 {
                        drained += popped;
                        drained2.store(drained, Ordering::Relaxed);
                    } else {
                        std::hint::spin_loop();
                    }
                } else {
                    let now = Instant::now();
                    let dt = now.duration_since(last).as_secs_f64();
                    last = now;
                    tokens = (tokens + dt * drain_rate as f64).min(bucket_max);
                    let mut popped = 0u64;
                    while tokens >= 1.0 {
                        if cons.pop().is_ok() {
                            tokens -= 1.0;
                            popped += 1;
                        } else {
                            tokens = 0.0; // ring empty: don't bank idle tokens
                            break;
                        }
                    }
                    if popped > 0 {
                        drained += popped;
                        drained2.store(drained, Ordering::Relaxed);
                    }
                    std::hint::spin_loop();
                }
            }
        })
        .expect("spawn drain");

    let symbols = cfg.symbols as u32;
    let users = cfg.users.max(2);
    let hot = cfg.hot_users.max(2);
    let mut seq: Vec<u64> = vec![0; cfg.symbols];
    let mut hist = new_hist();

    let base_t = Instant::now();
    let max_ns = cfg.max_ms * 1_000_000;
    let start = now_ns(base_t);
    let mut bp_engaged = false;
    let mut first_bp_ns: u64 = 0;
    let mut bp_accum_ns: u64 = 0;
    let mut bp_since: u64 = 0;
    let mut prev_bp = false;
    let mut done: u64 = 0;

    // process_fill pushes PER_FILL persist events per fill (Fill + taker
    // position+account + maker position+account + Tip = 6, all users in-shard
    // here). The real WAL.md rule is "persist ring full -> stall the hot path"
    // (tick() returns early while backpressured and does not process a message it
    // cannot fully persist). We mirror that by gating on ROOM FOR A WHOLE FILL
    // BEFORE calling process_fill, using SELF-ACCOUNTED occupancy
    // (pushed - drained) so a partial 6-event burst never pushes a few events
    // then drops the rest while still counting the fill as done -- that both
    // corrupts state and perversely inflates achieved/s as the drain slows
    // (oracle finding #1, the inversion). Gating up-front makes achieved/s
    // correctly track drain_rate / PER_FILL.
    const PER_FILL: u64 = 6;
    let occ_full = cap as u64;
    // HYSTERESIS watermarks. Gating per-fill on "room for one more fill" causes a
    // lockstep ping-pong with the drain thread's publish cadence (engine resumes
    // one fill, immediately re-reads a stale drained count, stalls again) which
    // throttles to the publish rate, not the drain rate. Instead: stall when the
    // ring fills to HIGH, and resume only once the drain has pulled occupancy back
    // down to LOW, then run free up to HIGH again. This is exactly the
    // fill-then-drain duty cycle of a real persist-ring + sidecar, and makes the
    // sustained achieved rate track the DRAIN rate, not a synchronization artifact.
    let hi = occ_full.saturating_sub(PER_FILL);
    let lo = occ_full / 2;
    let mut pushed: u64 = 0;
    while done < fills {
        let occ_now = pushed.saturating_sub(drained_ct.load(Ordering::Relaxed));
        if prev_bp {
            // currently stalled: stay stalled until the drain pulls occ <= LOW.
            let now = now_ns(base_t);
            if now.saturating_sub(start) >= max_ns {
                break;
            }
            if occ_now <= lo {
                bp_accum_ns += now.saturating_sub(bp_since);
                shard.backpressured = false;
                prev_bp = false;
            } else {
                std::hint::spin_loop();
                continue;
            }
        } else if occ_now >= hi {
            // ring just hit the high watermark: enter the stall.
            let now = now_ns(base_t);
            if !bp_engaged {
                bp_engaged = true;
                first_bp_ns = now.saturating_sub(start);
            }
            bp_since = now;
            prev_bp = true;
            shard.backpressured = true;
            std::hint::spin_loop();
            continue;
        }

        let sid = (done % symbols as u64) as u32;
        seq[sid as usize] += 1;
        let s = seq[sid as usize];
        let taker = (done % hot as u64) as u32;
        let maker = (taker + 1) % users;
        let f = FillEvent {
            seq: s,
            symbol_id: sid,
            taker_user_id: taker,
            maker_user_id: maker,
            price: MARK,
            qty: QTY,
            taker_side: (s & 1) as u8,
            timestamp_ns: done,
        };
        let t0 = now_ns(base_t);
        shard.process_fill(&f);
        let t1 = now_ns(base_t);
        hist.record((t1 - t0).max(1)).ok();
        pushed += PER_FILL;
        let occ = pushed.saturating_sub(drained_ct.load(Ordering::Relaxed));
        if occ > hwm.load(Ordering::Relaxed) {
            hwm.store(occ, Ordering::Relaxed);
        }
        done += 1;
        if t1.saturating_sub(start) >= max_ns {
            break;
        }
    }
    if prev_bp {
        bp_accum_ns += now_ns(base_t).saturating_sub(bp_since);
    }
    let elapsed = now_ns(base_t).saturating_sub(start);
    stop.store(true, Ordering::Relaxed);
    drain.join().ok();

    FillFloodResult {
        achieved: done as f64 / (elapsed as f64 / 1e9),
        p99_ns: hist.value_at_quantile(0.99),
        backpressure_engaged: bp_engaged,
        time_to_first_bp_ms: first_bp_ns as f64 / 1e6,
        bp_total_ms: bp_accum_ns as f64 / 1e6,
        persist_hwm: hwm.load(Ordering::Relaxed) as usize,
        completed: done,
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

/// Pin the engine/pacer thread so OS scheduling jitter does not contaminate the
/// sub-capacity latency tail (oracle code finding B). Best effort.
fn pin_self() {
    if let Some(cores) = core_affinity::get_core_ids() {
        if let Some(c) = cores.last() {
            core_affinity::set_for_current(*c);
        }
    }
}

fn main() {
    pin_self();
    let cfg = load_cfg();
    println!("=== rsx-risk FLOOD bench (behaviour vs offered load) ===");
    println!(
        "users={} symbols={} hot_users={} depth={} orders/rate={} max={}ms warmup={}",
        cfg.users, cfg.symbols, cfg.hot_users, cfg.depth, cfg.orders, cfg.max_ms, cfg.warmup,
    );
    println!("Engine-only (no UDP/decode), single shard. Latency = completion - SCHEDULED-due time");
    println!("(open-loop, coordinated-omission-free). Knee = first rate where achieved/offered<0.95.");
    println!();
    println!("--- Table A: ORDER FLOOD (process_order, pre-trade margin + freeze) ---");
    println!(
        "{:>12} {:>12} {:>7} {:>9} {:>9} {:>9} {:>9} {:>10} {:>5}",
        "offered/s", "achieved/s", "a/o%", "p50 us", "p99 us", "p999 us", "max us", "backlog", "knee",
    );

    let orders = build_orders(&cfg, (cfg.orders.min(4_000_000) as usize).max(cfg.warmup as usize + 1));
    let mut knee_found = false;
    for &rate in &cfg.rates {
        let r = order_flood_at_rate(&cfg, &orders, rate);
        let ao = r.achieved / r.offered as f64;
        let p99_us = r.hist.value_at_quantile(0.99) as f64 / 1000.0;
        // Knee = first rate where the engine can no longer keep the offered
        // schedule (sustained achieved/offered < 0.95). The earlier p99-ratio
        // criterion fired a FALSE POSITIVE against a noisy low-rate baseline
        // (oracle code finding A) -- p99 stays a column, it does not pick the knee.
        let knee = !knee_found && ao < 0.95 && r.completed > 1000;
        if knee {
            knee_found = true;
        }
        println!(
            "{:>12} {:>12.0} {:>6.1}% {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>10} {:>5}",
            rate,
            r.achieved,
            ao * 100.0,
            r.hist.value_at_quantile(0.50) as f64 / 1000.0,
            p99_us,
            r.hist.value_at_quantile(0.999) as f64 / 1000.0,
            r.hist.max() as f64 / 1000.0,
            r.max_backlog,
            if knee { "<==" } else { "" },
        );
    }
    println!();
    println!("Read: a/o% (achieved/offered) ~100% below capacity. The KNEE row is where the engine");
    println!("can no longer keep the schedule -- a/o drops and scheduled-latency p99 climbs; past it,");
    println!("achieved plateaus while backlog grows. NOTE: this plateau is the flood-HARNESS ceiling");
    println!("(per op it also paces the schedule + times + records a histogram), so it sits BELOW the");
    println!("pure engine ceiling reported by risk_throughput_bench order-accept. Same shape, lower");
    println!("absolute number -- use throughput for the ceiling, this for the degradation curve.");

    if cfg.skip_fill {
        return;
    }
    println!();
    println!("--- Table B: FILL FLOOD + PERSIST BACKPRESSURE (drain rate swept) ---");
    println!("process_fill pushes ~6 persist events/fill (Fill + taker pos/acct + maker pos/acct +");
    println!("Tip) onto a 16384-slot ring; drain throttled. Engine STALLS (does not drop) when full.");
    println!(
        "{:>14} {:>12} {:>9} {:>6} {:>12} {:>11} {:>11}",
        "drain ev/s", "achieved/s", "p99 us", "bp?", "1st-bp ms", "bp-time ms", "persist hwm",
    );
    let fills = cfg.orders.min(2_000_000);
    for &dr in &cfg.fill_rates {
        let r = fill_flood(&cfg, dr, fills);
        let drain_lbl = if dr == u64::MAX { "unthrottled".to_string() } else { format!("{}", dr) };
        println!(
            "{:>14} {:>12.0} {:>9.3} {:>6} {:>12.2} {:>11.2} {:>11}",
            drain_lbl,
            r.achieved,
            r.p99_ns as f64 / 1000.0,
            if r.backpressure_engaged { "yes" } else { "no" },
            r.time_to_first_bp_ms,
            r.bp_total_ms,
            r.persist_hwm,
        );
        std::hint::black_box(r.completed);
    }
    println!();
    println!("Read: achieved fill/s tracks drain_rate / ~6 (each fill needs ~6 persist slots): the");
    println!("hot path STALLS when the ring fills and resumes when the sidecar drains it (the WAL.md");
    println!("persist-ring-full stall), so a slower sidecar directly throttles the fill engine -- NOT");
    println!("the inversion of dropping events to fake higher throughput. NOTE the 'unthrottled'/fast");
    println!("rows do NOT reach the pure process_fill CPU rate: they are bounded by the cross-core");
    println!("SPSC persist HANDOFF ceiling in THIS harness (~0.7M events/s = ~115k fills/s on a");
    println!("6-core dev box, one producer + one consumer bouncing the ring head/tail cache line).");
    println!("In production the persist consumer is the tokio PG sidecar, whose real drain rate is the");
    println!("throttled rows -- so read those as the operative backpressure regime, and the throughput");
    println!("bench's fill row (no persist producer = pure engine CPU) for the engine-side cost.");
}
