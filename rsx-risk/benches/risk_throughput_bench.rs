//! Risk-engine CAPACITY bench: max sustained req/s + per-op SERVICE time.
//! ====================================================================
//!
//! THE QUESTION
//! ------------
//! "How many requests per second can ONE risk shard process?" This drives the
//! real hot path -- `RiskShard::process_order` (pre-trade margin check + freeze)
//! and `RiskShard::process_fill` (position/fee update + persist fan-out) -- in a
//! tight single-thread closed loop and reports the ops/s ceiling plus the
//! per-op cost distribution.
//!
//! WHAT THIS IS (and what it is NOT)
//! ---------------------------------
//! This is the risk ENGINE service ceiling, measured by direct method calls.
//! It is single-shard, in-process. There is NO UDP / WS / casting recv / decode
//! here -- that is the gateway's concern and a separate transport budget. The
//! production recv loop calls `process_order`/`process_fill` directly off one
//! net buffer (input rings were removed), so the only thing excluded vs prod is
//! the casting recv + one decode-memcpy per message. Treat the number as
//! "engine CPU ceiling, transport excluded" -- stated, not hidden.
//!
//! Because the loop is single-thread and closed (no queue), the per-op timer is
//! the SERVICE TIME, not latency-under-load. p50/p99 here are CPU jitter of the
//! work itself, NOT load-induced tail. We label them `service ns` deliberately.
//! The load-induced tail + the knee live in `risk_flood_bench`. ops/s here
//! cross-checks the flood bench's saturation point. (Oracle design critique #2.)
//!
//! THE HIDDEN VARIABLE: open-order cardinality
//! -------------------------------------------
//! `process_order` calls `frozen_for_user(user)` which sums the user's OPEN
//! orders (O(this user's resting orders), per-user index). A bench with ~0 open
//! orders per user understates production cost where active users rest many
//! orders. So we SWEEP resting-order depth per active user (the `depth` column)
//! and report ops/s as a function of it. (Oracle design critique #3.)
//!
//! REJECT CLASSES ARE NOT ONE BUCKET
//! ---------------------------------
//! `NotInShard` exits before any margin math (cheap); `InsufficientMargin` runs
//! the full `calculate` + `check_order` then rejects (as expensive as an
//! accept, minus the freeze insert). We measure pure-accept, pure-NotInShard,
//! and pure-InsufficientMargin SEPARATELY, then a realistic mixed order stream.
//! (Oracle design critique #4.)
//!
//! WORKLOADS
//! ---------
//!   order-accept     : 100% accepting orders, swept over resting-order depth.
//!   order-reject-shard: 100% NotInShard (cheap early-exit path).
//!   order-reject-margin: 100% InsufficientMargin (full calc, then reject).
//!   order-mixed      : 95% accept / 3% NotInShard / 2% InsufficientMargin.
//!   fill             : 100% process_fill, fresh seqs, NO persist producer
//!                      attached (push is a no-op) so the number is pure engine
//!                      CPU, not bounded by the cross-core persist drain (that
//!                      drain + its backpressure live in risk_flood_bench Table
//!                      B). Hot-user skew option (a small set takes most fills).
//!   mixed-stream     : 4 orders : 1 fill interleaved (many orders, fewer fills).
//!
//! METHOD
//! ------
//! Per workload: warm up (`WARMUP_MS`), then run a fixed wall window
//! (`WINDOW_MS`) doing as many ops as possible; ops/s = ops / window. Latency is
//! SAMPLED (1 in `SAMPLE_EVERY`) into an hdrhistogram so per-op `Instant::now()`
//! overhead does not distort the ops/s number -- the timer cost is calibrated
//! and printed. Inputs are pre-built; no allocation in the measured loop. Each
//! workload runs `REPS` times; we report median ops/s + min/max spread.
//!
//! ENV KNOBS (all optional)
//!   RTB_USERS=10000        seeded accounts (all in shard; shard_count=1)
//!   RTB_SYMBOLS=16         tradeable symbols
//!   RTB_DEPTHS="0,8,64,512" resting-order-per-active-user depths to sweep
//!   RTB_HOT_USERS=64       active users that actually trade (hot set)
//!   RTB_WINDOW_MS=2000     measured window per rep
//!   RTB_WARMUP_MS=300      warm-up before each rep
//!   RTB_REPS=5             repetitions (report median + spread)
//!   RTB_SAMPLE_EVERY=16    record 1 in N latencies
//!
//! CAVEATS (read before quoting)
//!   * Engine-only: excludes casting recv + decode (one memcpy/msg) + UDP send.
//!   * Single shard, single core. Real deployment runs many shards in parallel;
//!     this is the PER-SHARD ceiling.
//!   * Cache-friendly cardinality by default (RTB_USERS=10k, RTB_SYMBOLS=16).
//!     Bump RTB_USERS / RTB_SYMBOLS to probe a colder regime; the default is the
//!     warm case and is labelled as such.
//!   * service p50/p99 are CPU jitter, not load tail -- see header.

use std::time::Duration;
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
use rsx_risk::types::FillEvent;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

struct Cfg {
    users: u32,
    symbols: usize,
    depths: Vec<usize>,
    hot_users: u32,
    window: Duration,
    warmup: Duration,
    reps: usize,
    sample_every: u64,
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(default)
}

fn load_cfg() -> Cfg {
    let depths = std::env::var("RTB_DEPTHS")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect::<Vec<usize>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![0, 8, 64, 512]);
    Cfg {
        users: env_u64("RTB_USERS", 10_000) as u32,
        symbols: env_u64("RTB_SYMBOLS", 16) as usize,
        depths,
        hot_users: env_u64("RTB_HOT_USERS", 64) as u32,
        window: Duration::from_millis(env_u64("RTB_WINDOW_MS", 2_000)),
        warmup: Duration::from_millis(env_u64("RTB_WARMUP_MS", 300)),
        reps: env_u64("RTB_REPS", 5) as usize,
        sample_every: env_u64("RTB_SAMPLE_EVERY", 16),
    }
}

const COLLATERAL: i64 = 1_000_000_000_000; // deep enough that accepts never starve
const MARK: i64 = 50_000;
const QTY: i64 = 10;

// ---------------------------------------------------------------------------
// Shard construction + seeding
// ---------------------------------------------------------------------------

fn make_shard(symbols: usize) -> RiskShard {
    let mut params = Vec::with_capacity(symbols);
    let mut taker = Vec::with_capacity(symbols);
    let mut maker = Vec::with_capacity(symbols);
    for _ in 0..symbols {
        params.push(SymbolRiskParams {
            initial_margin_rate: 1000, // 10%
            maintenance_margin_rate: 500,
            max_leverage: 10,
        });
        taker.push(5i64);
        maker.push(-1i64);
    }
    let mut shard = RiskShard::new(ShardConfig {
        shard_id: 0,
        shard_count: 1, // every user_id % 1 == 0 -> all in shard
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

fn seed_accounts(shard: &mut RiskShard, users: u32) {
    for uid in 0..users {
        shard.accounts.insert(uid, Account::new(uid, COLLATERAL));
    }
}

/// Pre-fill the shard so each hot user already rests `depth` orders (populates
/// frozen_orders / frozen_by_user so frozen_for_user has real work to sum).
/// These resting orders are NOT released during the measured loop.
fn seed_resting_orders(shard: &mut RiskShard, hot_users: u32, symbols: usize, depth: usize) {
    let mut oid: u64 = 1;
    for u in 0..hot_users {
        for d in 0..depth {
            let sid = (d % symbols) as u32;
            let o = order_template(u, sid, oid);
            // process_order inserts the freeze on accept; leave it resting.
            let _ = shard.process_order(&o);
            oid += 1;
        }
    }
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

fn fill_template(seq: u64, symbol_id: u32, taker: u32, maker: u32) -> FillEvent {
    FillEvent {
        seq,
        symbol_id,
        taker_user_id: taker,
        maker_user_id: maker,
        price: MARK,
        qty: QTY,
        taker_side: (seq & 1) as u8,
        timestamp_ns: seq, // monotone, used by liquidation-check now_ns
    }
}

// ---------------------------------------------------------------------------
// Latency timer overhead calibration
// ---------------------------------------------------------------------------

fn calibrate_timer_ns() -> f64 {
    let iters = 200_000u64;
    let mut acc = 0u64;
    let t0 = Instant::now();
    for _ in 0..iters {
        acc = acc.wrapping_add(Instant::now().elapsed().as_nanos() as u64);
    }
    let e = t0.elapsed().as_nanos() as f64 / iters as f64;
    std::hint::black_box(acc);
    e
}

// ---------------------------------------------------------------------------
// Per-rep result
// ---------------------------------------------------------------------------

struct RepResult {
    ops: u64,
    secs: f64,
    hist: Histogram<u64>,
}

impl RepResult {
    fn ops_per_sec(&self) -> f64 {
        self.ops as f64 / self.secs
    }
}

fn new_hist() -> Histogram<u64> {
    // 1 ns .. 60 ms, 3 sig figs.
    Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap()
}

// ---------------------------------------------------------------------------
// Workload runners. Each takes a fully-seeded shard and a closure that does ONE
// op, returns nothing; we time a sampled subset and run for `window`.
// ---------------------------------------------------------------------------

/// Run `op` in a closed loop for `window`, sampling latency 1-in-N.
/// `op` returns () and advances its own state via the &mut captured env.
fn run_window<F: FnMut()>(mut op: F, warmup: Duration, window: Duration, sample_every: u64) -> RepResult {
    // warmup
    let w0 = Instant::now();
    while w0.elapsed() < warmup {
        for _ in 0..1024 {
            op();
        }
    }
    let mut hist = new_hist();
    let mut ops: u64 = 0;
    let t0 = Instant::now();
    // Check the clock every CHECK ops to keep clock overhead off the hot loop.
    const CHECK: u64 = 256;
    loop {
        for _ in 0..CHECK {
            if ops.is_multiple_of(sample_every) {
                let s = Instant::now();
                op();
                let dt = s.elapsed().as_nanos() as u64;
                hist.record(dt.max(1)).ok();
            } else {
                op();
            }
            ops += 1;
        }
        if t0.elapsed() >= window {
            break;
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    RepResult { ops, secs, hist }
}

// ---------------------------------------------------------------------------
// ORDER workloads
// ---------------------------------------------------------------------------

/// Accepting orders over a hot-user set at a fixed resting depth. To stay in
/// steady state we keep a sliding window of WINDOW open orders: release the one
/// submitted WINDOW ops ago. This keeps the per-user open-order count bounded at
/// ~depth + (WINDOW/hot_users) instead of growing unbounded.
fn bench_order_accept(cfg: &Cfg, depth: usize) -> RepResult {
    let mut shard = make_shard(cfg.symbols);
    seed_accounts(&mut shard, cfg.users);
    seed_resting_orders(&mut shard, cfg.hot_users, cfg.symbols, depth);

    let hot = cfg.hot_users.max(1);
    let symbols = cfg.symbols as u32;
    const SLIDE: u64 = 1024;
    let mut oid: u64 = 100_000_000; // above any resting oid
    let base = oid;
    let mut i: u64 = 0;
    run_window(
        || {
            let u = (i % hot as u64) as u32;
            let sid = (i % symbols as u64) as u32;
            let cur = oid;
            let o = order_template(u, sid, cur);
            let _ = shard.process_order(&o);
            // release the order from SLIDE iterations ago to bound growth.
            if i >= SLIDE {
                let old = base + (i - SLIDE);
                let ou = ((i - SLIDE) % hot as u64) as u32;
                shard.release_frozen_for_order(ou, 0, old);
            }
            oid += 1;
            i += 1;
        },
        cfg.warmup,
        cfg.window,
        cfg.sample_every,
    )
}

/// 100% NotInShard: shard_count=1 means user_id % 1 == 0 always in shard, so we
/// force NotInShard with a shard whose count is 2 and id 0, feeding odd users.
fn bench_order_reject_shard(cfg: &Cfg) -> RepResult {
    // shard 0 of 2 -> odd users are NotInShard (cheap early exit).
    let mut shard = rebuild_with_shard_count(cfg.symbols, 2);
    for sid in 0..cfg.symbols {
        shard.mark_prices[sid] = MARK;
    }
    let symbols = cfg.symbols as u32;
    let mut i: u64 = 1; // odd user ids -> NotInShard
    run_window(
        || {
            let u = (2 * i + 1) as u32; // always odd
            let sid = (i % symbols as u64) as u32;
            let o = order_template(u, sid, i);
            let _ = shard.process_order(&o);
            i += 1;
        },
        cfg.warmup,
        cfg.window,
        cfg.sample_every,
    )
}

fn rebuild_with_shard_count(symbols: usize, shard_count: u32) -> RiskShard {
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
    RiskShard::new(ShardConfig {
        shard_id: 0,
        shard_count,
        max_symbols: symbols,
        symbol_params: params,
        taker_fee_bps: taker,
        maker_fee_bps: maker,
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig::default(),
    })
}

/// 100% InsufficientMargin: a user with ~zero collateral runs the full margin
/// calc and is rejected (not a cheap early exit).
fn bench_order_reject_margin(cfg: &Cfg) -> RepResult {
    let mut shard = make_shard(cfg.symbols);
    // one broke user, in shard
    shard.accounts.insert(7, Account::new(7, 1)); // collateral=1, can't afford IM
    let symbols = cfg.symbols as u32;
    let mut i: u64 = 0;
    run_window(
        || {
            let sid = (i % symbols as u64) as u32;
            let o = order_template(7, sid, i + 1);
            let _ = shard.process_order(&o);
            i += 1;
        },
        cfg.warmup,
        cfg.window,
        cfg.sample_every,
    )
}

/// Realistic order stream: 95% accept / 3% NotInShard / 2% InsufficientMargin.
///
/// The accept stream gets its OWN monotone index (`ai`) so the sliding release
/// always targets a real accepted order: keying release off the total stream
/// index leaks freezes whenever a release slot lands on a reject branch, which
/// grows frozen_by_user unbounded and tanks the number (oracle code finding #2).
/// The broke (InsufficientMargin) user lives ABOVE the hot accept set so the
/// 95/3/2 mix is not corrupted by the broke user also resting accept orders.
fn bench_order_mixed(cfg: &Cfg, depth: usize) -> RepResult {
    let mut shard = rebuild_with_shard_count(cfg.symbols, 2); // shard 0 of 2
    for sid in 0..cfg.symbols {
        shard.mark_prices[sid] = MARK;
    }
    // seed even users (in shard) with deep collateral.
    for uid in (0..cfg.users).step_by(2) {
        shard.accounts.insert(uid, Account::new(uid, COLLATERAL));
    }
    // broke user: even (in shard) but ABOVE the hot accept set, collateral=1.
    let broke: u32 = (cfg.hot_users + 4) * 2;
    shard.accounts.insert(broke, Account::new(broke, 1));
    // resting depth on hot even users
    {
        let mut oid: u64 = 1;
        for k in 0..cfg.hot_users {
            let u = k * 2; // even
            for d in 0..depth {
                let sid = (d % cfg.symbols) as u32;
                let o = order_template(u, sid, oid);
                let _ = shard.process_order(&o);
                oid += 1;
            }
        }
    }
    let hot = cfg.hot_users.max(1);
    let symbols = cfg.symbols as u32;
    const SLIDE: u64 = 1024;
    let base: u64 = 100_000_000;
    let mut i: u64 = 0;
    let mut ai: u64 = 0; // accept-only index (drives the sliding release)
    run_window(
        || {
            let r = i % 100;
            let sid = (i % symbols as u64) as u32;
            if r < 3 {
                // NotInShard: odd user (id grows but is rejected before any map touch)
                let o = order_template(2 * (i as u32) + 1, sid, 1);
                let _ = shard.process_order(&o);
            } else if r < 5 {
                // InsufficientMargin: broke user, full calc then reject
                let o = order_template(broke, sid, 1);
                let _ = shard.process_order(&o);
            } else {
                // accept: hot even user, own index so release always hits an accept
                let u = ((ai % hot as u64) as u32) * 2;
                let oid = base + ai;
                let o = order_template(u, sid, oid);
                let _ = shard.process_order(&o);
                if ai >= SLIDE {
                    let old = base + (ai - SLIDE);
                    let ou = (((ai - SLIDE) % hot as u64) as u32) * 2;
                    shard.release_frozen_for_order(ou, 0, old);
                }
                ai += 1;
            }
            i += 1;
        },
        cfg.warmup,
        cfg.window,
        cfg.sample_every,
    )
}

// ---------------------------------------------------------------------------
// FILL workload (no persist producer attached -- pure engine CPU; see note)
// ---------------------------------------------------------------------------

/// NO persist producer is attached, on purpose. `push_persist` is a no-op when
/// the producer is None, so this measures process_fill's CPU cost (dedup,
/// apply_fill x2, fee, liquidation check, AND the construction + clones of the
/// ~6 PersistEvents -- those still happen at the call sites) MINUS the ~6 SPSC
/// ring pushes and all drain/backpressure.
/// Why exclude the pushes: process_fill emits ~6 persist events/fill; at
/// multi-MHz that is >10M events/s, far above the cross-core SPSC drain ceiling
/// (~0.7M ev/s on this box). A real drain thread would fill the ring, push would
/// start FAILING, and process_fill would DROP events while still counting the
/// fill -- inflating the number with dropped work (the very inversion the flood
/// bench guards against). The SPSC pushes and the drain/backpressure behaviour
/// are measured honestly in risk_flood_bench Table B. Here: engine CPU only.
fn bench_fill(cfg: &Cfg, hot_skew: bool) -> RepResult {
    let mut shard = make_shard(cfg.symbols);
    seed_accounts(&mut shard, cfg.users);

    let symbols = cfg.symbols as u32;
    let users = cfg.users.max(2);
    let hot = cfg.hot_users.max(2);
    let mut seq: Vec<u64> = vec![0; cfg.symbols]; // per-symbol monotone seq
    let mut i: u64 = 0;
    run_window(
        || {
            let sid = (i % symbols as u64) as u32;
            seq[sid as usize] += 1;
            let s = seq[sid as usize];
            let (taker, maker) = if hot_skew {
                // small hot set takes most fills
                (((i) % hot as u64) as u32, ((i + 1) % hot as u64) as u32)
            } else {
                (((i) % users as u64) as u32, ((i + 1) % users as u64) as u32)
            };
            let f = fill_template(s, sid, taker, maker.max(taker + 1) % users);
            shard.process_fill(&f);
            i += 1;
        },
        cfg.warmup,
        cfg.window,
        cfg.sample_every,
    )
}

// ---------------------------------------------------------------------------
// MIXED stream: 4 orders : 1 fill
// ---------------------------------------------------------------------------

/// No persist producer attached (same rationale as bench_fill): the fill leg
/// would otherwise drop persist events once the cross-core drain falls behind.
/// This measures the engine CPU of the interleaved order+fill stream.
fn bench_mixed_stream(cfg: &Cfg, depth: usize) -> RepResult {
    let mut shard = make_shard(cfg.symbols);
    seed_accounts(&mut shard, cfg.users);
    seed_resting_orders(&mut shard, cfg.hot_users, cfg.symbols, depth);

    let hot = cfg.hot_users.max(2);
    let symbols = cfg.symbols as u32;
    let users = cfg.users.max(2);
    const SLIDE: u64 = 1024;
    let base: u64 = 100_000_000;
    let mut seq: Vec<u64> = vec![0; cfg.symbols];
    let mut oi: u64 = 0; // order index
    let mut i: u64 = 0; // total op index
    run_window(
        || {
            if i % 5 == 4 {
                // fill
                let sid = (i % symbols as u64) as u32;
                seq[sid as usize] += 1;
                let s = seq[sid as usize];
                let taker = (i % hot as u64) as u32;
                let maker = (taker + 1) % users;
                let f = fill_template(s, sid, taker, maker);
                shard.process_fill(&f);
            } else {
                // order (accept)
                let u = (oi % hot as u64) as u32;
                let sid = (oi % symbols as u64) as u32;
                let oid = base + oi;
                let o = order_template(u, sid, oid);
                let _ = shard.process_order(&o);
                if oi >= SLIDE {
                    let old = base + (oi - SLIDE);
                    let ou = ((oi - SLIDE) % hot as u64) as u32;
                    shard.release_frozen_for_order(ou, 0, old);
                }
                oi += 1;
            }
            i += 1;
        },
        cfg.warmup,
        cfg.window,
        cfg.sample_every,
    )
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

fn median_spread(reps: &[RepResult]) -> (f64, f64, f64, &RepResult) {
    let mut rates: Vec<f64> = reps.iter().map(|r| r.ops_per_sec()).collect();
    rates.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = rates[rates.len() / 2];
    let min = rates[0];
    let max = rates[rates.len() - 1];
    // pick the rep whose rate is closest to median for the histogram report
    let mut best = &reps[0];
    let mut bestd = f64::MAX;
    for r in reps {
        let d = (r.ops_per_sec() - med).abs();
        if d < bestd {
            bestd = d;
            best = r;
        }
    }
    (med, min, max, best)
}

fn run_reps<F: FnMut() -> RepResult>(reps: usize, mut f: F) -> Vec<RepResult> {
    (0..reps).map(|_| f()).collect()
}

fn print_row(label: &str, reps: &[RepResult]) {
    let (med, min, max, h) = median_spread(reps);
    println!(
        "{:<26} {:>12.0} {:>10.0} {:>10.0} {:>9} {:>9} {:>9} {:>9}",
        label,
        med,
        min,
        max,
        h.hist.value_at_quantile(0.50),
        h.hist.value_at_quantile(0.99),
        h.hist.value_at_quantile(0.999),
        h.hist.max(),
    );
}

/// Pin the measuring thread to the last core so OS scheduling jitter does not
/// pollute the per-op service-time tail (oracle code finding B). Best effort.
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
    let timer_ns = calibrate_timer_ns();

    println!("=== rsx-risk THROUGHPUT (capacity) bench ===");
    println!(
        "users={} symbols={} hot_users={} window={}ms warmup={}ms reps={} sample=1/{}",
        cfg.users,
        cfg.symbols,
        cfg.hot_users,
        cfg.window.as_millis(),
        cfg.warmup.as_millis(),
        cfg.reps,
        cfg.sample_every,
    );
    println!("Instant::now()+elapsed overhead ~= {:.1} ns/sample (latency cols include this)", timer_ns);
    println!("NOTE: ops/s is the engine SERVICE ceiling (no transport/decode/UDP). Latency cols");
    println!("      are per-op SERVICE time (no queue) -- load tail + knee are in risk_flood_bench.");
    println!();
    println!(
        "{:<26} {:>12} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9}",
        "workload", "ops/s(med)", "ops/s min", "ops/s max", "p50 ns", "p99 ns", "p999 ns", "max ns",
    );

    // ORDER accept, swept over resting-order depth.
    for &depth in &cfg.depths {
        let reps = run_reps(cfg.reps, || bench_order_accept(&cfg, depth));
        print_row(&format!("order-accept depth={}", depth), &reps);
    }
    // Reject classes.
    {
        let reps = run_reps(cfg.reps, || bench_order_reject_shard(&cfg));
        print_row("order-reject NotInShard", &reps);
    }
    {
        let reps = run_reps(cfg.reps, || bench_order_reject_margin(&cfg));
        print_row("order-reject InsufMargin", &reps);
    }
    // Realistic mix (use the smallest non-zero depth, or 8).
    {
        let depth = *cfg.depths.iter().find(|&&d| d > 0).unwrap_or(&8);
        let reps = run_reps(cfg.reps, || bench_order_mixed(&cfg, depth));
        print_row(&format!("order-mixed depth={}", depth), &reps);
    }
    // FILL workloads.
    {
        let reps = run_reps(cfg.reps, || bench_fill(&cfg, false));
        print_row("fill uniform-users", &reps);
    }
    {
        let reps = run_reps(cfg.reps, || bench_fill(&cfg, true));
        print_row("fill hot-users", &reps);
    }
    // MIXED stream 4:1.
    {
        let depth = *cfg.depths.iter().find(|&&d| d > 0).unwrap_or(&8);
        let reps = run_reps(cfg.reps, || bench_mixed_stream(&cfg, depth));
        print_row(&format!("mixed 4ord:1fill d={}", depth), &reps);
    }

    println!();
    println!("Read: ops/s(med) is the per-shard capacity for that workload. order-accept rises in");
    println!("cost (falls in ops/s) as resting-order depth grows -- frozen_for_user sums the user's");
    println!("open orders each pre-trade check. NotInShard is the cheap early-exit floor.");
}
