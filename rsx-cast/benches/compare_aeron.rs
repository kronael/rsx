//! Aeron loopback round-trip comparison.
//!
//! `aeron_rtt_udp_loopback` — Aeron over UDP unicast, ping/pong on the same
//!   host. Two clients share one embedded media driver: PING publishes to
//!   `ping_channel`, PONG subscribes to it and re-publishes on `pong_channel`.
//!   PING subscribes to `pong_channel` and times the round-trip. Payload =
//!   128 bytes (matches `FillRecord` size used in cmp_rtt_bench).
//!
//! `aeron_rtt_ipc` — Aeron IPC (shared-memory broadcast), same shape but
//!   bypasses the UDP socket entirely. Lower bound on Aeron's overhead;
//!   useful as a control to isolate UDP cost from media driver cost.
//!
//! What this measures
//! ------------------
//! Criterion times the full `record_rtt()` closure on the PING side:
//!   write timestamp into the payload, spin in `offer()` until the publication
//!   accepts, spin in `subscription.poll()` until the echo handler returns,
//!   read `last_rtt_ns` from the handler.
//!
//! The handler-recorded `last_rtt_ns` (computed from the embedded timestamp)
//! is a separate, finer-grained measurement of the wire round-trip and is
//! intentionally not what Criterion reports — Criterion's number includes
//! the offer/poll spin overhead, the FFI call boundary, and the callback
//! dispatch. We expect handler RTT ≤ Criterion RTT.
//!
//! What this does NOT measure
//! --------------------------
//! - Media-driver startup (driver launched in `setup()`, drained before the
//!   timed loop starts).
//! - Pub/sub connection establishment (Aeron publishes the SETUP frame +
//!   exchanges SM/heartbeat before the first user message; we warm up
//!   for 100 iterations before recording).
//! - JVM warmup / GC pauses — there is no JVM. `rusteron-media-driver`
//!   uses the precompiled C media driver via FFI.
//!
//! Caveats
//! -------
//! - The PING thread (Criterion timer) is pinned to core 2 and the PONG
//!   echo thread to core 3. Media-driver agents are NOT pinned — Aeron
//!   spawns its conductor + sender + receiver agents internally and we
//!   don't have hooks to pin them. On a 6-core slice the agents float
//!   on cores 0/1/4/5; PING and PONG are isolated from them. Without
//!   pinning, oversubscription on a small box (<8 cores) inflates the
//!   UDP RTT by an order of magnitude vs. published numbers (~21 µs P50
//!   on c6in.16xlarge per Real Logic). See compare/aeron.md.
//! - Aeron's media driver runs as a background agent thread inside this
//!   process (`AeronDriver::launch_embedded`). Production deployments use
//!   a separate driver process; the IPC hop between application and driver
//!   over shared memory is the same in both cases.
//! - 128-byte payload. Aeron prepends a 32-byte data frame header → ~160 B
//!   on the wire. CMP's 128-byte payload + 16-byte WalHeader → 144 B on the
//!   wire. Header overhead is ~11% higher for Aeron at this payload size.
//! - **Not apples-to-apples vs CMP at the transport layer.** CMP RTT covers
//!   `app → kernel UDP → app`. Aeron UDP RTT covers
//!   `app → driver shm → kernel UDP → driver shm → app`, both directions.
//!   The IPC bench (`aeron_rtt_ipc_64b`) bypasses the kernel entirely and
//!   is the closest like-for-like overhead comparison.
//!
//! See compare/aeron.md for full protocol analysis and published numbers.

use core_affinity::CoreId;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use rusteron_client::Aeron;
use rusteron_client::AeronBufferClaim;
use rusteron_client::AeronContext;
use rusteron_client::AeronFragmentHandlerCallback;
use rusteron_client::AeronHeader;
use rusteron_client::AeronPublication;
use rusteron_client::AeronSubscription;
use rusteron_client::Handler;
use rusteron_client::Handlers;
use rusteron_media_driver::AeronDriver;
use rusteron_media_driver::AeronDriverContext;
use rusteron_media_driver::IntoCString;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const PING_STREAM_ID: i32 = 1002;
const PONG_STREAM_ID: i32 = 1003;
const FRAGMENT_LIMIT: usize = 10;

/// Cores 2 (PING/timer) + 3 (PONG/echo). Aeron's internal agents
/// (conductor / sender / receiver) are NOT pinned by this — they
/// run wherever the C-side `idle_strategy` thread spawning lands.
fn pick_cores() -> (CoreId, CoreId) {
    let ids = core_affinity::get_core_ids().unwrap_or_default();
    let p = ids.get(2).copied().unwrap_or(CoreId { id: 0 });
    let e = ids.get(3).copied().unwrap_or(CoreId { id: 1 });
    (p, e)
}
/// 128-byte payload matches `cmp_rtt_bench` (size_of::<FillRecord>() == 128
/// per rsx-messages/src/lib.rs:78). First 8 bytes carry the timestamp; the
/// rest is zeroed.
const PAYLOAD_LEN: usize = 128;
const WARMUP_ITERS: usize = 100;

// ── echo handler used on the PONG side ──────────────────────────────────────

struct PongHandler {
    publisher: AeronPublication,
    buffer_claim: AeronBufferClaim,
}

impl AeronFragmentHandlerCallback for PongHandler {
    #[inline]
    fn handle_aeron_fragment_handler(&mut self, buffer: &[u8], header: AeronHeader) {
        // Preserve flags so SBE/header semantics match across the echo.
        let flags = header.get_values().expect("aeron header").frame.flags;
        // Spin until we can claim a contiguous region in the term buffer.
        // try_claim is zero-copy: we write the payload directly into the
        // outgoing term buffer.
        while self.publisher.try_claim(buffer.len(), &self.buffer_claim) < 0 {
            std::hint::spin_loop();
        }
        self.buffer_claim.frame_header_mut().flags = flags;
        self.buffer_claim.data_mut().copy_from_slice(buffer);
        self.buffer_claim.commit().expect("aeron commit");
    }
}

// ── timing handler used on the PING side ────────────────────────────────────

struct PingHandler {
    last_rtt_ns: i64,
}

impl AeronFragmentHandlerCallback for PingHandler {
    #[inline]
    fn handle_aeron_fragment_handler(&mut self, buffer: &[u8], _header: AeronHeader) {
        let t0 = i64::from_le_bytes(buffer[0..8].try_into().expect("8-byte ts"));
        self.last_rtt_ns = Aeron::nano_clock() - t0;
        debug_assert!(self.last_rtt_ns >= 0);
    }
}

// ── shared setup for both UDP and IPC variants ──────────────────────────────

struct AeronRig {
    // Held to keep the PING-side client alive for the duration of the bench.
    // FFI handles drop the underlying Aeron resources when this is dropped.
    _aeron: Aeron,
    pong_subscription: AeronSubscription,
    ping_publication: AeronPublication,
    /// Signals the PONG echo thread to exit.
    pong_stop: Arc<AtomicBool>,
    /// Signals the embedded media driver thread to exit. Set inside Drop.
    driver_stop: Arc<AtomicBool>,
    /// Joined inside Drop to guarantee the driver doesn't outlive the rig
    /// and contaminate the next bench in the criterion_group.
    driver_handle: Option<thread::JoinHandle<Result<(), rusteron_media_driver::AeronCError>>>,
    pong_handle: Option<thread::JoinHandle<()>>,
}

impl Drop for AeronRig {
    fn drop(&mut self) {
        // Order matters:
        //   1. Stop the PONG echo thread so it stops touching pub/sub.
        //   2. Stop the embedded media driver agent.
        //   3. Let `Aeron` / `pub` / `sub` drop in the order their owning
        //      fields are declared (Rust drops fields top-to-bottom).
        //      `_aeron` is declared above `pong_subscription` and
        //      `ping_publication`, so the client conductor goes first and
        //      the resource handles drop afterward — matching the C-side
        //      lifecycle (close client, then close transport handles).
        self.pong_stop.store(true, Ordering::Release);
        if let Some(h) = self.pong_handle.take() {
            let _ = h.join();
        }
        self.driver_stop.store(true, Ordering::Release);
        if let Some(h) = self.driver_handle.take() {
            let _ = h.join();
        }
    }
}

fn setup(ping_channel: &str, pong_channel: &str, pong_core: CoreId) -> AeronRig {
    // Unique driver directory per bench so concurrent runs (and CI) don't
    // collide on /dev/shm/aeron-$USER.
    let ctx = AeronDriverContext::new().expect("driver context");
    let unique_dir = format!("{}-{}", ctx.get_dir(), Aeron::nano_clock());
    ctx.set_dir(&unique_dir.clone().into_c_string()).expect("set_dir");
    ctx.set_dir_delete_on_start(true).expect("delete_on_start");
    ctx.set_dir_delete_on_shutdown(true).expect("delete_on_shutdown");

    let (driver_stop, driver_handle) = AeronDriver::launch_embedded(ctx.clone(), false);

    // Driver is up. PING client connects.
    let cli_ctx = AeronContext::new().expect("client context");
    cli_ctx.set_dir(&unique_dir.clone().into_c_string()).expect("client set_dir");
    cli_ctx.set_idle_sleep_duration_ns(0).expect("zero idle sleep");
    let aeron = Aeron::new(&cli_ctx).expect("client new");
    aeron.start().expect("client start");

    // PING publishes on ping_channel, subscribes on pong_channel.
    let ping_publication = aeron
        .async_add_publication(&ping_channel.into_c_string(), PING_STREAM_ID)
        .expect("add ping pub")
        .poll_blocking(Duration::from_secs(5))
        .expect("ping pub ready");
    let pong_subscription = aeron
        .async_add_subscription(
            &pong_channel.into_c_string(),
            PONG_STREAM_ID,
            Handlers::no_available_image_handler(),
            Handlers::no_unavailable_image_handler(),
        )
        .expect("add pong sub")
        .poll_blocking(Duration::from_secs(5))
        .expect("pong sub ready");

    // PONG thread: separate client, subscribes to ping_channel, echoes to
    // pong_channel. Shared media driver via the same `unique_dir`.
    let pong_dir = unique_dir.clone();
    let pong_ping_channel = ping_channel.to_string();
    let pong_pong_channel = pong_channel.to_string();
    let pong_stop = Arc::new(AtomicBool::new(false));
    let pong_stop_thread = pong_stop.clone();
    let pong_ready = Arc::new(AtomicBool::new(false));
    let pong_ready_thread = pong_ready.clone();
    let pong_handle = thread::Builder::new()
        .name("aeron-pong".into())
        .spawn(move || {
            core_affinity::set_for_current(pong_core);
            let ctx = AeronContext::new().expect("pong context");
            ctx.set_dir(&pong_dir.into_c_string()).expect("pong set_dir");
            ctx.set_idle_sleep_duration_ns(0).expect("pong zero idle");
            let aeron = Aeron::new(&ctx).expect("pong aeron");
            aeron.start().expect("pong start");
            let pong_pub = aeron
                .async_add_publication(&pong_pong_channel.into_c_string(), PONG_STREAM_ID)
                .expect("pong pub")
                .poll_blocking(Duration::from_secs(5))
                .expect("pong pub ready");
            let ping_sub = aeron
                .async_add_subscription(
                    &pong_ping_channel.into_c_string(),
                    PING_STREAM_ID,
                    Handlers::no_available_image_handler(),
                    Handlers::no_unavailable_image_handler(),
                )
                .expect("ping sub")
                .poll_blocking(Duration::from_secs(5))
                .expect("ping sub ready");
            let handler = Handler::leak(PongHandler {
                publisher: pong_pub,
                buffer_claim: AeronBufferClaim::default(),
            });
            // Signal: PONG endpoints are open. PING side can stop waiting.
            pong_ready_thread.store(true, Ordering::Release);
            while !pong_stop_thread.load(Ordering::Acquire) {
                let _ = ping_sub.poll(Some(&handler), FRAGMENT_LIMIT);
            }
        })
        .expect("spawn pong");

    // Wait for PONG side to advertise both endpoints.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pong_ready.load(Ordering::Acquire) {
        if Instant::now() > deadline {
            panic!("pong thread failed to come up");
        }
        thread::sleep(Duration::from_millis(10));
    }

    // Aeron publications need a connected image before offer() succeeds;
    // poll until both sides are connected.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ping_publication.is_connected() || !pong_subscription.is_connected() {
        if Instant::now() > deadline {
            panic!("pub/sub never connected");
        }
        std::hint::spin_loop();
    }

    AeronRig {
        _aeron: aeron,
        pong_subscription,
        ping_publication,
        pong_stop,
        driver_stop,
        driver_handle: Some(driver_handle),
        pong_handle: Some(pong_handle),
    }
}

fn record_rtt(
    publication: &AeronPublication,
    subscription: &AeronSubscription,
    buffer: &mut [u8],
    handler: &mut Handler<PingHandler>,
) -> i64 {
    let now = Aeron::nano_clock();
    buffer[0..8].copy_from_slice(&now.to_le_bytes());
    // offer() spins on backpressure (slow subscriber, term full).
    while publication.offer(buffer, Handlers::no_reserved_value_supplier_handler()) < 0 {
        std::hint::spin_loop();
    }
    // Spin on poll until the echo lands. PingHandler stamps last_rtt_ns.
    loop {
        let n = subscription
            .poll(Some(handler), FRAGMENT_LIMIT)
            .unwrap_or_default();
        if n > 0 {
            break;
        }
        std::hint::spin_loop();
    }
    handler.last_rtt_ns
}

// ── UDP loopback bench ──────────────────────────────────────────────────────

fn bench_aeron_udp(c: &mut Criterion) {
    let (ping_core, pong_core) = pick_cores();
    let rig = setup(
        "aeron:udp?endpoint=127.0.0.1:40123",
        "aeron:udp?endpoint=127.0.0.1:40124",
        pong_core,
    );

    // Pin the PING thread (Criterion's timer thread).
    core_affinity::set_for_current(ping_core);

    let mut buffer = vec![0u8; PAYLOAD_LEN];
    let mut handler = Handler::leak(PingHandler { last_rtt_ns: 0 });

    // Warmup: prime the term buffers + JIT-free FFI paths.
    for _ in 0..WARMUP_ITERS {
        let _ = record_rtt(&rig.ping_publication, &rig.pong_subscription, &mut buffer, &mut handler);
    }

    c.bench_function("aeron_rtt_udp_loopback_128b", |b| {
        b.iter(|| {
            let rtt = record_rtt(
                &rig.ping_publication,
                &rig.pong_subscription,
                &mut buffer,
                &mut handler,
            );
            black_box(rtt);
        });
    });

    // Drop order: handler first (releases the leaked PingHandler box),
    // then rig (signals + joins both PONG and the embedded driver).
    drop(handler);
    drop(rig);
}

// ── IPC (shared-memory) bench ───────────────────────────────────────────────
//
// Kept in source as a documented variant — running it in the same process
// after the UDP bench triggers a C-side conductor race ("MediaDriver has
// been shutdown"). Wire it into the criterion_group below to run it
// standalone (delete `bench_aeron_udp` from `targets`).
#[allow(dead_code)]
fn bench_aeron_ipc(c: &mut Criterion) {
    let (ping_core, pong_core) = pick_cores();
    // aeron:ipc bypasses the UDP socket — pure shared-memory broadcast.
    // Useful baseline: isolates Aeron's protocol overhead from the UDP
    // sendto/recvfrom cost.
    let rig = setup("aeron:ipc", "aeron:ipc", pong_core);
    core_affinity::set_for_current(ping_core);

    let mut buffer = vec![0u8; PAYLOAD_LEN];
    let mut handler = Handler::leak(PingHandler { last_rtt_ns: 0 });

    for _ in 0..WARMUP_ITERS {
        let _ = record_rtt(&rig.ping_publication, &rig.pong_subscription, &mut buffer, &mut handler);
    }

    c.bench_function("aeron_rtt_ipc_128b", |b| {
        b.iter(|| {
            let rtt = record_rtt(
                &rig.ping_publication,
                &rig.pong_subscription,
                &mut buffer,
                &mut handler,
            );
            black_box(rtt);
        });
    });

    drop(handler);
    drop(rig);
}

// Only the UDP bench runs in this binary's criterion_group. The IPC bench
// is in a sibling binary (`compare_aeron_ipc.rs`) because running both
// driver lifecycles in one process triggers a C-side
// "MediaDriver has been shutdown" race: the second `launch_embedded`
// reuses conductor globals before the first driver's SHM tear-down has
// fully settled. Running them as two separate cargo bench targets avoids
// the in-process collision.
//
// Run both with:
//   cargo bench -p rsx-cast --bench compare_aeron       # UDP loopback
//   cargo bench -p rsx-cast --bench compare_aeron_ipc   # IPC (shared mem)
criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50);
    targets = bench_aeron_udp
}
criterion_main!(benches);
