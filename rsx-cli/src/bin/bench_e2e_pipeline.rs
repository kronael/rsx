//! In-process GW→ME→GW round-trip bench.
//!
//! Wires a real ME (Orderbook + WalWriter + DedupTracker +
//! order index) plus REAL CmpSender/CmpReceiver pairs over
//! loopback UDP, in a single process. Drives N orders through
//! the same code paths as production ME, but with no cross-
//! process scheduling jitter — this measures the algorithmic
//! floor of GW→ME→GW.
//!
//! Topology (all on 127.0.0.1):
//!
//!   "Gateway" thread (this main) → CmpSender → 127.0.0.1:GW_PORT
//!                                                        ↓ UDP
//!     "ME" thread → CmpReceiver bind GW_PORT
//!                 → process_new_order + WAL
//!                 → CmpSender → 127.0.0.1:GW_ECHO_PORT
//!                                          ↓ UDP
//!   "Gateway" thread → CmpReceiver bind GW_ECHO_PORT
//!                    → record taker_ts_ns → now() delta
//!
//! Risk validation (margin check) is intentionally out of
//! scope here — the parallel sub owns rsx-risk and tested
//! it in isolation. This bench measures the ME critical
//! section plus two CMP UDP hops, which is the gap between
//! "algorithmic floor" and "cross-process measurement".
//!
//! Output:
//!   bench-e2e-pipeline --n 10000
//!
//!   round-trip (us) p50=... p95=... p99=...
//!
//! Compare against the cross-process production p50 in
//! bench-baseline.json (currently 1128 us).

use clap::Parser;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::protocol::CmpRecord;
use rsx_dxs::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_matching::wire::OrderMessage;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::time::time_ns;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;
use rustc_hash::FxHashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const SYMBOL_ID: u32 = 42;

/// OrderMessage wrapper that implements CmpRecord so we can
/// send it via CmpSender::send. The on-wire layout matches
/// what rsx-matching/src/wire.rs reads with ptr::read_unaligned,
/// and the record_type matches what the ME main loop dispatches
/// on (RECORD_ORDER_REQUEST).
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct OrderRequestWire {
    inner: OrderMessage,
}

impl CmpRecord for OrderRequestWire {
    fn seq(&self) -> u64 {
        self.inner.seq
    }
    fn set_seq(&mut self, seq: u64) {
        self.inner.seq = seq;
    }
    fn record_type() -> u16 {
        RECORD_ORDER_REQUEST
    }
}

#[derive(Parser, Debug)]
#[command(name = "bench-e2e-pipeline")]
struct Args {
    /// Number of orders to drive through the pipeline.
    #[arg(long, default_value_t = 10_000)]
    n: u64,
    /// Loopback port for Gateway→ME (orders).
    #[arg(long, default_value_t = 39_100)]
    me_port: u16,
    /// Loopback port for ME→Gateway (fills/order_done).
    #[arg(long, default_value_t = 39_200)]
    gw_echo_port: u16,
    /// Warmup orders to discard from the histogram.
    #[arg(long, default_value_t = 500)]
    warmup: u64,
}

fn symbol_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYMBOL_ID,
        price_decimals: 2,
        qty_decimals: 4,
        tick_size: 1,
        lot_size: 1,
    }
}

fn percentile(samples: &mut [u64], p: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let idx = ((samples.len() as f64) * p) as usize;
    samples[idx.min(samples.len() - 1)]
}

fn make_book_with_liquidity() -> Orderbook {
    let mut book = Orderbook::new(
        symbol_config(),
        65_536,
        100_000,
    );
    // Resting asks at 100_001..100_050 so each new taker
    // bid at 100_001 fills exactly one level.
    for i in 0..50 {
        let mut ask = IncomingOrder {
            price: 100_001 + i as i64,
            qty: 1_000_000,
            remaining_qty: 1_000_000,
            side: Side::Sell,
            tif: TimeInForce::GTC,
            user_id: 200 + i as u32,
            reduce_only: false,
            post_only: false,
            timestamp_ns: 1_000,
            order_id_hi: 0,
            order_id_lo: 5_000 + i as u64,
        };
        process_new_order(&mut book, &mut ask);
    }
    book
}

fn me_loop(
    me_port: u16,
    gw_echo_port: u16,
    wal_dir: PathBuf,
    shutdown: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
) {
    let me_bind: SocketAddr =
        format!("127.0.0.1:{me_port}").parse().unwrap();
    let gw_echo: SocketAddr =
        format!("127.0.0.1:{gw_echo_port}").parse().unwrap();
    // Gateway's listening port on the NAK return path — we
    // don't expect NAKs in this bench, but the constructor
    // wants a target. Use gw_echo_port; NAKs from ME would
    // be sent here harmlessly.
    let nak_target = gw_echo;

    let mut recv = CmpReceiver::new(
        me_bind,
        nak_target,
        SYMBOL_ID,
    )
    .expect("ME CmpReceiver bind");

    let mut sender = CmpSender::new(
        gw_echo,
        SYMBOL_ID,
        &wal_dir,
    )
    .expect("ME CmpSender");

    let mut wal = WalWriter::new(
        SYMBOL_ID,
        &wal_dir,
        None,
        64 * 1024 * 1024,
        0,
    )
    .expect("wal writer");

    let mut book = make_book_with_liquidity();
    let mut dedup = DedupTracker::new();
    let mut index: FxHashMap<(u32, u64, u64), u32> =
        FxHashMap::default();

    ready.store(true, Ordering::SeqCst);

    while !shutdown.load(Ordering::SeqCst) {
        if let Some((hdr, payload)) = recv.try_recv() {
            if hdr.record_type != RECORD_ORDER_REQUEST
                || payload.len()
                    < std::mem::size_of::<OrderMessage>()
            {
                continue;
            }
            // SAFETY: same pattern as ME main; payload is
            // wire-format OrderMessage.
            let order_msg = unsafe {
                std::ptr::read_unaligned(
                    payload.as_ptr() as *const OrderMessage,
                )
            };

            let is_dup = dedup.check_and_insert(
                order_msg.user_id,
                order_msg.order_id_hi,
                order_msg.order_id_lo,
            );
            if is_dup {
                continue;
            }

            let ts_accept = time_ns();
            let mut accepted = OrderAcceptedRecord {
                seq: 0,
                ts_ns: ts_accept,
                user_id: order_msg.user_id,
                symbol_id: SYMBOL_ID,
                order_id_hi: order_msg.order_id_hi,
                order_id_lo: order_msg.order_id_lo,
                price: order_msg.price,
                qty: order_msg.qty,
                side: order_msg.side,
                tif: order_msg.tif,
                reduce_only: order_msg.reduce_only,
                post_only: order_msg.post_only,
                cid: [0; 20],
            };
            wal.append(&mut accepted).unwrap();

            let mut incoming = order_msg.to_incoming();
            process_new_order(&mut book, &mut incoming);

            let ts_ev = time_ns();
            write_events_to_wal(
                &mut wal, &book, SYMBOL_ID, ts_ev,
            )
            .unwrap();

            for event in book.events() {
                match *event {
                    rsx_book::event::Event::OrderInserted {
                        user_id,
                        side,
                        price,
                        qty,
                        order_id_hi,
                        order_id_lo,
                        handle,
                    } => {
                        index.insert(
                            (
                                user_id,
                                order_id_hi,
                                order_id_lo,
                            ),
                            handle,
                        );
                        let mut rec = OrderInsertedRecord {
                            seq: 0,
                            ts_ns: ts_ev,
                            symbol_id: SYMBOL_ID,
                            user_id,
                            order_id_hi,
                            order_id_lo,
                            price,
                            qty,
                            side,
                            reduce_only: 0,
                            tif: 0,
                            post_only: 0,
                            _pad1: [0; 4],
                        };
                        let _ = sender.send(&mut rec);
                    }
                    rsx_book::event::Event::Fill {
                        maker_user_id,
                        taker_user_id,
                        price,
                        qty,
                        side,
                        maker_order_id_hi,
                        maker_order_id_lo,
                        taker_order_id_hi,
                        taker_order_id_lo,
                        taker_ts_ns,
                        ..
                    } => {
                        let mut rec = FillRecord {
                            seq: 0,
                            ts_ns: ts_ev,
                            symbol_id: SYMBOL_ID,
                            taker_user_id,
                            maker_user_id,
                            _pad0: 0,
                            taker_order_id_hi,
                            taker_order_id_lo,
                            maker_order_id_hi,
                            maker_order_id_lo,
                            price,
                            qty,
                            taker_side: side,
                            reduce_only: 0,
                            tif: 0,
                            post_only: 0,
                            _pad1: [0; 4],
                            taker_ts_ns,
                        };
                        let _ = sender.send(&mut rec);
                    }
                    rsx_book::event::Event::OrderDone {
                        user_id,
                        order_id_hi,
                        order_id_lo,
                        filled_qty,
                        remaining_qty,
                        reason,
                        ..
                    } => {
                        index.remove(&(
                            user_id,
                            order_id_hi,
                            order_id_lo,
                        ));
                        let mut rec = OrderDoneRecord {
                            seq: 0,
                            ts_ns: ts_ev,
                            symbol_id: SYMBOL_ID,
                            user_id,
                            order_id_hi,
                            order_id_lo,
                            filled_qty,
                            remaining_qty,
                            final_status: reason,
                            reduce_only: 0,
                            tif: 0,
                            post_only: 0,
                            _pad1: [0; 4],
                        };
                        let _ = sender.send(&mut rec);
                    }
                    _ => {}
                }
            }
        }
        recv.tick();
        let _ = sender.tick();
        sender.recv_control();
    }
}

fn main() {
    let args = Args::parse();
    let total = args.n + args.warmup;

    let tmp = PathBuf::from("./tmp/bench_e2e_pipeline");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let shutdown = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));

    let me_port = args.me_port;
    let gw_echo_port = args.gw_echo_port;
    let me_tmp = tmp.clone();
    let me_shutdown = shutdown.clone();
    let me_ready = ready.clone();
    let me_handle = thread::spawn(move || {
        me_loop(
            me_port,
            gw_echo_port,
            me_tmp,
            me_shutdown,
            me_ready,
        );
    });

    // Wait for ME to bind.
    let wait_start = Instant::now();
    while !ready.load(Ordering::SeqCst) {
        if wait_start.elapsed() > Duration::from_secs(5) {
            eprintln!("ME failed to start in 5s");
            std::process::exit(1);
        }
        thread::sleep(Duration::from_millis(10));
    }

    // Gateway side: send orders, await fills.
    let me_addr: SocketAddr =
        format!("127.0.0.1:{me_port}").parse().unwrap();
    let gw_bind: SocketAddr =
        format!("127.0.0.1:{gw_echo_port}").parse().unwrap();

    let gw_tmp = tmp.join("gw");
    std::fs::create_dir_all(&gw_tmp).unwrap();

    let mut to_me = CmpSender::new(
        me_addr,
        SYMBOL_ID,
        &gw_tmp,
    )
    .expect("GW CmpSender");

    let mut from_me = CmpReceiver::new(
        gw_bind,
        me_addr,
        SYMBOL_ID,
    )
    .expect("GW CmpReceiver");

    let mut samples_us: Vec<u64> =
        Vec::with_capacity(total as usize);
    let pending = std::collections::HashMap::<u64, u64>::new();
    let mut pending = pending;

    let bench_start = Instant::now();

    // Strict request/reply: send one order, busy-poll until
    // its terminal event arrives, then send the next. This
    // measures sequential round-trip latency (the metric the
    // production e2e_us figure also represents).
    for i in 0..total {
        let oid_lo = i + 1;
        let send_ts = time_ns();
        let order = OrderMessage {
            seq: 0,
            price: 100_001,
            qty: 1,
            side: 0, // Buy
            tif: 1,  // IOC
            reduce_only: 0,
            post_only: 0,
            _pad1: [0; 4],
            user_id: 999,
            _pad2: 0,
            timestamp_ns: send_ts,
            order_id_hi: 0,
            order_id_lo: oid_lo,
        };
        let mut wire = OrderRequestWire { inner: order };
        loop {
            match to_me.send(&mut wire) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
        pending.insert(oid_lo, send_ts);

        // Busy-poll for terminal event referencing oid_lo.
        let wait_start = Instant::now();
        let mut terminal = false;
        while !terminal {
            let _ = to_me.tick();
            to_me.recv_control();
            while let Some((hdr, payload)) =
                from_me.try_recv()
            {
                let mut got_term_oid: Option<u64> = None;
                match hdr.record_type {
                    RECORD_FILL => {
                        if payload.len()
                            >= std::mem::size_of::<
                                FillRecord,
                            >()
                        {
                            let rec = unsafe {
                                std::ptr::read_unaligned(
                                    payload.as_ptr()
                                        as *const FillRecord,
                                )
                            };
                            got_term_oid = Some(
                                rec.taker_order_id_lo,
                            );
                        }
                    }
                    RECORD_ORDER_DONE => {
                        if payload.len()
                            >= std::mem::size_of::<
                                OrderDoneRecord,
                            >()
                        {
                            let rec = unsafe {
                                std::ptr::read_unaligned(
                                    payload.as_ptr()
                                        as *const OrderDoneRecord,
                                )
                            };
                            got_term_oid = Some(
                                rec.order_id_lo,
                            );
                        }
                    }
                    _ => {}
                }
                if let Some(seen_oid) = got_term_oid {
                    let now = time_ns();
                    if let Some(t0) =
                        pending.remove(&seen_oid)
                    {
                        samples_us
                            .push((now - t0) / 1000);
                    }
                    if seen_oid == oid_lo {
                        terminal = true;
                    }
                }
            }
            if wait_start.elapsed()
                > Duration::from_millis(500)
            {
                eprintln!(
                    "timeout waiting for oid={oid_lo}"
                );
                break;
            }
        }

        if bench_start.elapsed() > Duration::from_secs(60) {
            eprintln!(
                "overall timeout at order {i}",
            );
            break;
        }
    }

    shutdown.store(true, Ordering::SeqCst);
    let _ = me_handle.join();

    // Drop warmup.
    let warmup = args.warmup as usize;
    if samples_us.len() > warmup {
        samples_us.drain(0..warmup);
    }

    let n_kept = samples_us.len();
    let mut s = samples_us.clone();
    let p50 = percentile(&mut s, 0.50);
    let p95 = percentile(&mut s, 0.95);
    let p99 = percentile(&mut s, 0.99);
    let min = s.first().copied().unwrap_or(0);
    let max = s.last().copied().unwrap_or(0);
    let total_elapsed_ms =
        bench_start.elapsed().as_millis();

    // Read cross-process p50 from bench-baseline.json if
    // present. Best-effort, no fail.
    let baseline = std::fs::read_to_string(
        "bench-baseline.json",
    )
    .ok()
    .and_then(|s| {
        serde_json::from_str::<serde_json::Value>(&s).ok()
    });
    let cross_p50_us =
        baseline.as_ref().and_then(|v| {
            v.get("e2e_us")
                .and_then(|e| e.get("p50"))
                .and_then(|p| p.as_f64())
        });

    println!("bench-e2e-pipeline (in-process)");
    println!("  samples kept: {n_kept}");
    println!("  elapsed ms:   {total_elapsed_ms}");
    println!("  round-trip us:");
    println!("    min   = {min}");
    println!("    p50   = {p50}");
    println!("    p95   = {p95}");
    println!("    p99   = {p99}");
    println!("    max   = {max}");
    if let Some(x) = cross_p50_us {
        let ratio = if p50 > 0 {
            x / p50 as f64
        } else {
            0.0
        };
        println!(
            "  cross-process p50 (from bench-baseline.json):"
        );
        println!("    p50_us  = {x:.1}");
        println!(
            "    speedup = {ratio:.2}x (cross/in-process)"
        );
    } else {
        println!(
            "  cross-process p50: bench-baseline.json not \
             readable; compare manually"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[allow(dead_code)]
fn _silence_unused() {
    let _ = Price(0);
    let _ = Qty(0);
}
