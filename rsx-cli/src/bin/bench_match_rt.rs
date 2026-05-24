//! `bench-match-rt` binary: in-process matching round-trip with per-stage timing.

use clap::Parser;
use rsx_book::book::Orderbook;
use rsx_book::matching::process_new_order;
use rsx_book::matching::IncomingOrder;
use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::records::CastRecord;
use rsx_cast::wal::WalWriter;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal_integration::write_events_to_wal;
use rsx_matching::wire::OrderMessage;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_types::Side;
use rsx_types::SymbolConfig;
use rsx_types::TimeInForce;

/// Thin newtype that lets a raw `OrderMessage` ride the
/// CastSender path. Same memory layout — `repr(C)` —
/// and `CastRecord` lets CMP set the seq.
#[repr(C)]
#[derive(Clone, Copy)]
struct OrderRequestWire {
    inner: OrderMessage,
}

impl CastRecord for OrderRequestWire {
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
use rustc_hash::FxHashMap;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

const SYMBOL_ID: u32 = 10;
const MID_PRICE: i64 = 100_000;

/// Stages, in order. `t[i+1] - t[i]` is the cost of the leg
/// labelled by stage names below.
const STAGE_NAMES: &[&str] = &[
    "gw_send",       // 0 → 1: gateway-side CastSender::send
    "udp_to_me",     // 1 → 2: UDP loopback + ME try_recv body
    "me_dedup",      // 2 → 3: dedup hashmap check + insert
    "me_wal_accept", // 3 → 4: OrderAccepted WAL append
    "me_match",      // 4 → 5: process_new_order
    "me_wal_events", // 5 → 6: write_events_to_wal
    "me_send",       // 6 → 7: ME-side CastSender::send (fill)
    "udp_to_gw",     // 7 → 8: UDP loopback + gw try_recv body
];

const N_STAGES: usize = 9; // 8 deltas

#[derive(Parser)]
#[command(about = "in-process matching round-trip latency + per-stage breakdown")]
struct Args {
    /// Number of measured orders.
    #[arg(long, default_value_t = 10_000)]
    n: usize,

    /// Warmup orders (discarded).
    #[arg(long, default_value_t = 500)]
    warmup: usize,
}

#[inline]
fn now_ns() -> u64 {
    // perf_counter_ns: monotonic, ~ns precision, ~25 ns per call.
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as u64
}

fn pick_port() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// ME-side timestamps for one iteration. Sent back to the
/// main thread via an mpsc channel and joined by `oid`.
struct MeStages {
    oid_lo: u64,
    t: [u64; 6], // me_recv, dedup_done, wal_accept_done, match_done, wal_events_done, me_send_done
}

fn main() {
    let args = Args::parse();
    let total = args.n + args.warmup;

    // Two WAL dirs, one per CastSender (ME and gateway each
    // keep their own NAK retransmit cache). Plain tmp dirs;
    // they leak after the bench exits — that's fine.
    let tmp_me = std::path::PathBuf::from("./tmp/bench_match_rt_me");
    let tmp_gw = std::path::PathBuf::from("./tmp/bench_match_rt_gw");
    let _ = std::fs::remove_dir_all(&tmp_me);
    let _ = std::fs::remove_dir_all(&tmp_gw);
    std::fs::create_dir_all(&tmp_me).unwrap();
    std::fs::create_dir_all(&tmp_gw).unwrap();

    // Four distinct UDP ports — one per socket. CastSender
    // and CastReceiver each own their own port; otherwise
    // SO_REUSEPORT lets the kernel hash-distribute incoming
    // packets between them and replies vanish.
    let gw_send_bind = pick_port();
    let gw_recv_bind = pick_port();
    let me_send_bind = pick_port();
    let me_recv_bind = pick_port();

    // gw_sender → me_recv_bind
    let mut gw_sender = CastSender::with_config(
        me_recv_bind,
        1,
        tmp_gw.as_path(),
        &rsx_cast::config::CastConfig {
            sender_bind_addr: Some(gw_send_bind.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    // gw_receiver listens on gw_recv_bind; sender_addr is
    // where it'd send NAKs (back at me_sender).
    let mut gw_receiver =
        CastReceiver::new(gw_recv_bind, me_send_bind).unwrap();

    // me_sender → gw_recv_bind
    let mut me_sender = CastSender::with_config(
        gw_recv_bind,
        2,
        tmp_me.as_path(),
        &rsx_cast::config::CastConfig {
            sender_bind_addr: Some(me_send_bind.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    // me_receiver listens on me_recv_bind; sender_addr is
    // where it'd send NAKs (back at gw_sender).
    let mut me_receiver =
        CastReceiver::new(me_recv_bind, gw_send_bind).unwrap();

    // Pre-populate the orderbook with N+warmup ask orders so
    // every gateway order has a maker to fill against. One
    // resting ask per probe iteration.
    let mut wal = WalWriter::new(
        SYMBOL_ID,
        tmp_me.as_path(),
        64 * 1024 * 1024, // 64 MB rotation
    )
    .unwrap();
    let mut book = make_book_with_liquidity(total);
    let mut dedup = DedupTracker::new();
    let mut order_index: FxHashMap<(u32, u64, u64), u32> =
        FxHashMap::default();

    // Channel for ME stages → main thread.
    let (me_tx, me_rx) = mpsc::channel::<MeStages>();

    // ME worker.
    let me = thread::spawn(move || {
        let mut tick_count: u64 = 0;
        loop {
            // Periodic tick → keep flow-control windows open.
            tick_count = tick_count.wrapping_add(1);
            if tick_count & 0x3FF == 0 {
                me_receiver.tick();
                let _ = me_sender.tick();
                me_sender.recv_control();
            }

            let (hdr, payload) = match me_receiver.try_recv() {
                CastRecv::Data(h, p) => (h, p),
                CastRecv::Empty => {
                    std::hint::spin_loop();
                    continue;
                }
                CastRecv::Faulted { .. } | CastRecv::Reconnect { .. } => {
                    // Bench harness: faulted/reconnect aborts the run.
                    return;
                }
            };
            let t0 = now_ns();
            // Exit sentinel: any payload of size 0 (we never
            // send such a thing in normal operation).
            if payload.is_empty() {
                return;
            }
            let order_msg = match decode_payload::<OrderMessage>(&payload) {
                Some(v) => v,
                None => continue,
            };
            let _ = hdr;
            let oid_lo = order_msg.order_id_lo;

            let is_dup = dedup.check_and_insert(
                order_msg.user_id,
                order_msg.order_id_hi,
                oid_lo,
            );
            let t1 = now_ns();
            if is_dup {
                continue;
            }

            let mut accepted = OrderAcceptedRecord {
                seq: 0,
                ts_ns: order_msg.timestamp_ns,
                user_id: order_msg.user_id,
                symbol_id: SYMBOL_ID,
                order_id_hi: order_msg.order_id_hi,
                order_id_lo: oid_lo,
                price: order_msg.price,
                qty: order_msg.qty,
                side: order_msg.side,
                tif: order_msg.tif,
                reduce_only: order_msg.reduce_only,
                post_only: order_msg.post_only,
                cid: [0; 20],
            };
            {
                let framed = wal.prepare(&mut accepted).unwrap();
                wal.append_framed(&framed).unwrap();
            }
            let t2 = now_ns();

            let mut incoming = order_msg.to_incoming();
            process_new_order(&mut book, &mut incoming);
            let t3 = now_ns();

            write_events_to_wal(&mut wal, &book, SYMBOL_ID, t3)
                .unwrap();
            for ev in book.events() {
                if let rsx_book::event::Event::OrderInserted {
                    handle,
                    user_id,
                    order_id_hi,
                    order_id_lo,
                    ..
                } = *ev
                {
                    order_index.insert(
                        (user_id, order_id_hi, order_id_lo),
                        handle,
                    );
                }
            }
            let t4 = now_ns();

            // Build + send a single fill back. (Real ME may
            // emit multiple events; for this bench we send
            // just one terminal fill record so the gateway
            // has something to await on.)
            let mut fill = FillRecord {
                seq: 0,
                ts_ns: order_msg.timestamp_ns,
                symbol_id: SYMBOL_ID,
                taker_user_id: order_msg.user_id,
                maker_user_id: 0,
                _pad0: 0,
                taker_order_id_hi: order_msg.order_id_hi,
                taker_order_id_lo: oid_lo,
                maker_order_id_hi: 0,
                maker_order_id_lo: 0,
                price: rsx_types::Price(order_msg.price),
                qty: rsx_types::Qty(order_msg.qty),
                taker_side: order_msg.side,
                reduce_only: 0,
                tif: 0,
                post_only: 0,
                _pad1: [0; 4],
                taker_ts_ns: order_msg.timestamp_ns,
            };
            if let Err(e) = me_sender.send(&mut fill) {
                panic!("me_sender: {e}");
            }
            let t5 = now_ns();

            // Send stages back. me_recv = t0; subsequent
            // entries are the cumulative timestamps so
            // the consumer can compute deltas trivially.
            me_tx
                .send(MeStages {
                    oid_lo,
                    t: [t0, t1, t2, t3, t4, t5],
                })
                .ok();
        }
    });

    // Gateway-side: send N orders, await each fill.
    let mut samples: Vec<[u64; N_STAGES]> =
        Vec::with_capacity(total);
    let mut gw_tick: u64 = 0;
    for i in 0..total {
        let oid_lo = i as u64 + 1; // skip 0 (sentinel)
        gw_tick = gw_tick.wrapping_add(1);
        if gw_tick & 0x3FF == 0 {
            gw_receiver.tick();
            let _ = gw_sender.tick();
            gw_sender.recv_control();
        }

        let ts = now_ns();
        let order = OrderMessage {
            user_id: 99,
            price: MID_PRICE + 100,
            qty: 1,
            side: Side::Buy as u8,
            tif: TimeInForce::IOC as u8,
            reduce_only: 0,
            post_only: 0,
            _pad1: [0; 4],
            _pad2: 0,
            timestamp_ns: ts,
            order_id_hi: 0,
            order_id_lo: oid_lo,
            seq: 0,
        };
        let mut wire = OrderRequestWire { inner: order };

        let s0 = now_ns(); // gw_send_start
        if let Err(e) = gw_sender.send(&mut wire) {
            panic!("gw_sender: {e}");
        }
        let s1 = now_ns(); // gw_send_done

        // Spin until the fill comes back. Per-order timeout
        // = 50 ms — generous; covers the WAL-fsync stall (10
        // ms flush window) and a couple of UDP drops. If we
        // hit it, drop the sample and continue rather than
        // hang the whole bench. Tick periodically inside the
        // wait so peer status round-trips stay live.
        let wait_start = std::time::Instant::now();
        let mut timed_out = false;
        let mut wait_tick: u64 = 0;
        loop {
            let (hdr, payload) = match gw_receiver.try_recv() {
                CastRecv::Data(h, p) => (h, p),
                CastRecv::Empty => {
                    wait_tick = wait_tick.wrapping_add(1);
                    if wait_tick & 0x3FF == 0 {
                        gw_receiver.tick();
                        let _ = gw_sender.tick();
                        gw_sender.recv_control();
                        if wait_start.elapsed()
                            > std::time::Duration::from_millis(50)
                        {
                            timed_out = true;
                            break;
                        }
                    }
                    std::hint::spin_loop();
                    continue;
                }
                CastRecv::Faulted { .. } | CastRecv::Reconnect { .. } => {
                    timed_out = true;
                    break;
                }
            };
            if hdr.record_type != RECORD_FILL {
                continue;
            }
            let fill = match decode_payload::<FillRecord>(&payload) {
                Some(v) => v,
                None => continue,
            };
            if fill.taker_order_id_lo != oid_lo {
                continue;
            }
            break;
        }
        let s8 = now_ns(); // gw_recv
        if timed_out {
            // Skip this sample (s[8] = 0 marks it incomplete;
            // print_report filters zero-deltas).
            continue;
        }

        // ME-side timestamps come via channel below; we'll
        // join later. Store only the gateway-visible slots
        // for now (idx 0, 1, 8).
        let mut s = [0u64; N_STAGES];
        s[0] = s0;
        s[1] = s1;
        s[8] = s8;
        samples.push(s);
    }

    // Tell ME to exit.
    let exit_fill = FillRecord {
        seq: 0,
        ts_ns: 0,
        symbol_id: SYMBOL_ID,
        taker_user_id: 0,
        maker_user_id: 0,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: 0,
        maker_order_id_hi: 0,
        maker_order_id_lo: 0,
        price: rsx_types::Price(0),
        qty: rsx_types::Qty(0),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        taker_ts_ns: 0,
    };
    let _ = exit_fill;
    // The ME thread doesn't have a clean exit sentinel in
    // this bench; we let it linger after main() returns.
    // Production binaries would set a flag.
    let _ = me;

    // Join ME stages by oid_lo (index 1..=total monotonic).
    let me_stages: FxHashMap<u64, MeStages> = me_rx
        .try_iter()
        .map(|m| (m.oid_lo, m))
        .collect();
    for (i, s) in samples.iter_mut().enumerate() {
        let oid_lo = i as u64 + 1;
        if let Some(me) = me_stages.get(&oid_lo) {
            s[2] = me.t[0]; // me_recv
            s[3] = me.t[1]; // me_dedup_done
            s[4] = me.t[2]; // me_wal_accept_done
            s[5] = me.t[3]; // me_match_done
            s[6] = me.t[4]; // me_wal_events_done
            s[7] = me.t[5]; // me_send_done
        }
    }

    // Discard warmup.
    samples.drain(..args.warmup);

    print_report(&samples);
}

fn make_book_with_liquidity(n_levels: usize) -> Orderbook {
    let cfg = SymbolConfig {
        symbol_id: SYMBOL_ID,
        price_decimals: 8,
        qty_decimals: 8,
        tick_size: 1,
        lot_size: 1,
    };
    // Slab capacity sized for the bench: one maker per
    // taker order + headroom. 64K is the same MAX_EVENTS
    // ceiling the ME uses; reusing it keeps the bench
    // cache-similar to the production matching engine.
    let book_cap = (n_levels as u32).next_power_of_two().max(8192);
    let mut book = Orderbook::new(cfg, book_cap, MID_PRICE);
    // Plenty of ask liquidity at MID + offset so the gateway
    // (buying at MID+100) crosses on every iteration.
    for i in 0..n_levels {
        let mut ask = IncomingOrder {
            price: MID_PRICE + 50,
            qty: 1,
            remaining_qty: 1,
            side: Side::Sell,
            tif: TimeInForce::GTC,
            user_id: 1,
            reduce_only: false,
            post_only: false,
            timestamp_ns: i as u64,
            order_id_hi: 0,
            order_id_lo: i as u64 + 1_000_000,
        };
        process_new_order(&mut book, &mut ask);
    }
    book
}

fn print_report(samples: &[[u64; N_STAGES]]) {
    println!("bench-match-rt  n={}", samples.len());
    println!();
    println!("Per-stage latency (ns):");
    println!(
        "{:<15} {:>10} {:>10} {:>10} {:>10}",
        "stage", "p50", "p95", "p99", "max",
    );

    let mut totals: Vec<u64> = Vec::with_capacity(samples.len());
    for (i, name) in STAGE_NAMES.iter().enumerate() {
        let mut deltas: Vec<u64> = samples
            .iter()
            .filter_map(|s| {
                if s[i] == 0 || s[i + 1] == 0 {
                    None
                } else {
                    Some(s[i + 1].saturating_sub(s[i]))
                }
            })
            .collect();
        deltas.sort_unstable();
        if deltas.is_empty() {
            continue;
        }
        let p50 = deltas[deltas.len() / 2];
        let p95 = deltas[deltas.len() * 95 / 100];
        let p99 = deltas[deltas.len() * 99 / 100];
        let max = *deltas.last().unwrap();
        println!(
            "{:<15} {:>10} {:>10} {:>10} {:>10}",
            name, p50, p95, p99, max,
        );
    }

    // Full round-trip = gw_send_start → gw_recv.
    for s in samples.iter() {
        if s[0] != 0 && s[N_STAGES - 1] != 0 {
            totals.push(s[N_STAGES - 1].saturating_sub(s[0]));
        }
    }
    totals.sort_unstable();
    if !totals.is_empty() {
        let p50 = totals[totals.len() / 2];
        let p95 = totals[totals.len() * 95 / 100];
        let p99 = totals[totals.len() * 99 / 100];
        let max = *totals.last().unwrap();
        println!();
        println!(
            "{:<15} {:>10} {:>10} {:>10} {:>10}",
            "TOTAL", p50, p95, p99, max,
        );
    }
}
