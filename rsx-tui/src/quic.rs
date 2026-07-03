//! QUIC client transport: `QuicConn` behind the `GatewayConn` trait.
//!
//! This is the user-facing client↔gateway leg. `GatewayConn` is
//! synchronous (the UI drains it non-blocking each render tick) but
//! quinn is async, so `QuicConn` owns a background tokio runtime on a
//! dedicated thread and bridges with channels:
//!
//! - `submit` pushes an `OrderReq` onto an unbounded channel; the async
//!   task drains it and writes an order frame.
//! - the async task reads event frames and pushes each `GwEvent` onto a
//!   std mpsc channel; `poll_event` drains it with `try_recv`.
//!
//! One connection, one bidirectional stream, one framed read loop. No
//! multiplexing. When `QuicConn` drops, the outbound channel closes, the
//! task finishes the stream and returns, and the runtime thread exits.
//!
//! Internal casting (rsx-cast) is a separate transport and is untouched.

use crate::conn::GatewayConn;
use crate::conn::GwEvent;
use crate::conn::OrderReq;
use crate::wire;
use quinn::ClientConfig;
use quinn::Connection;
use quinn::Endpoint;
use rustls::pki_types::CertificateDer;
use rustls::RootCertStore;
use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;
use tokio::runtime::Builder;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;

/// Build a root store trusting exactly the given DER certificates.
///
/// Production supplies the gateway's issuing certificate(s); the
/// loopback test supplies the self-signed cert `rcgen` generated.
pub fn roots(
    certs: impl IntoIterator<Item = CertificateDer<'static>>,
) -> io::Result<RootCertStore> {
    let mut store = RootCertStore::empty();
    for cert in certs {
        store
            .add(cert)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    }
    Ok(store)
}

/// A live QUIC connection to the gateway, drained by the UI each tick.
pub struct QuicConn {
    out: UnboundedSender<OrderReq>,
    inbound: Receiver<GwEvent>,
    /// Held so the runtime thread lives as long as the connection. It
    /// exits on its own when `out` drops, so it is never joined.
    _thread: JoinHandle<()>,
}

impl QuicConn {
    /// Connect to `server_addr`, validating its certificate against
    /// `roots` and its name against `server_name`. Returns immediately;
    /// the connection is established on the background thread and a
    /// `GwEvent::Connected` is delivered once the stream is open.
    pub fn connect(
        server_addr: SocketAddr,
        server_name: impl Into<String>,
        roots: RootCertStore,
    ) -> io::Result<Self> {
        let server_name = server_name.into();
        let (out, out_rx) = unbounded_channel::<OrderReq>();
        let (in_tx, inbound) = std::sync::mpsc::channel::<GwEvent>();
        let thread = std::thread::Builder::new()
            .name("rsx-tui-quic".to_owned())
            .spawn(move || {
                run_thread(server_addr, server_name, roots, out_rx, in_tx)
            })
            .map_err(io::Error::other)?;
        Ok(QuicConn { out, inbound, _thread: thread })
    }
}

impl GatewayConn for QuicConn {
    fn submit(&mut self, order: OrderReq) -> io::Result<()> {
        self.out.send(order).map_err(|_| {
            io::Error::new(io::ErrorKind::NotConnected, "quic link down")
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
    server_addr: SocketAddr,
    server_name: String,
    roots: RootCertStore,
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
    rt.block_on(run_client(
        server_addr,
        server_name,
        roots,
        out_rx,
        inbound,
    ));
}

/// The async client: dial, open one bi stream, then pump orders out and
/// events in until either side closes. Pushes `Connected` on stream
/// open and `Disconnected` on any failure or close.
async fn run_client(
    server_addr: SocketAddr,
    server_name: String,
    roots: RootCertStore,
    mut out_rx: UnboundedReceiver<OrderReq>,
    inbound: Sender<GwEvent>,
) {
    // quinn uses whatever rustls CryptoProvider is installed as default;
    // mirror the workspace's aws-lc-rs choice. Idempotent across conns.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let cfg = match ClientConfig::with_root_certificates(Arc::new(roots)) {
        Ok(cfg) => cfg,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    // Endpoint must outlive the connection; keep it bound for the fn.
    let mut endpoint = match Endpoint::client(bind_addr(server_addr)) {
        Ok(ep) => ep,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    endpoint.set_default_client_config(cfg);

    let conn = match dial(&endpoint, server_addr, &server_name).await {
        Ok(conn) => conn,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    let (mut send, mut recv) = match conn.open_bi().await {
        Ok(stream) => stream,
        Err(_) => {
            let _ = inbound.send(GwEvent::Disconnected);
            return;
        }
    };
    if inbound.send(GwEvent::Connected).is_err() {
        return;
    }

    // Send timestamps of in-flight orders, oldest first. A server
    // `Latency` frame (which carries internal + engine time) is paired
    // FIFO with the matching order to derive the net (client↔gateway)
    // leg the server can't see: net = measured RTT − internal − engine.
    // The pairing is exact only one-order-at-a-time (the frame carries
    // no oid); it is bounded to MAX_PENDING so an order that never
    // yields a `Latency` can only shift the window, never leak. When the
    // queue is empty or the subtraction underflows, `net` is left `None`
    // (rendered "—") rather than a fabricated 0.
    const MAX_PENDING: usize = 256;
    let mut pending: VecDeque<Instant> = VecDeque::new();

    loop {
        tokio::select! {
            maybe = out_rx.recv() => match maybe {
                Some(order) => {
                    if wire::write_order(&mut send, &order).await.is_err() {
                        let _ = inbound.send(GwEvent::Disconnected);
                        return;
                    }
                    if pending.len() >= MAX_PENDING {
                        pending.pop_front();
                    }
                    pending.push_back(Instant::now());
                }
                // QuicConn dropped: flush and exit cleanly.
                None => {
                    let _ = send.finish();
                    return;
                }
            },
            event = wire::read_event(&mut recv) => match event {
                Ok(ev) => {
                    let ev = fill_net_leg(ev, &mut pending);
                    if inbound.send(ev).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = inbound.send(GwEvent::Disconnected);
                    return;
                }
            },
        }
    }
}

/// For a server `Latency` frame, measure the round-trip against the
/// oldest in-flight order and fill the `net` leg (RTT minus the
/// server-reported internal + engine time). `net` stays `None` — never a
/// fabricated 0 — when there is no order to pair against or the measured
/// RTT is smaller than the server-reported time (clock skew / noise).
/// Other events pass through unchanged.
fn fill_net_leg(
    ev: GwEvent,
    pending: &mut VecDeque<Instant>,
) -> GwEvent {
    match ev {
        GwEvent::Latency { internal_ns, engine_ns, .. } => {
            let net_ns = pending.pop_front().and_then(|t| {
                let rtt_ns = t.elapsed().as_nanos() as u64;
                rtt_ns.checked_sub(internal_ns.saturating_add(engine_ns))
            });
            GwEvent::Latency { net_ns, internal_ns, engine_ns }
        }
        other => other,
    }
}

async fn dial(
    endpoint: &Endpoint,
    server_addr: SocketAddr,
    server_name: &str,
) -> io::Result<Connection> {
    let connecting = endpoint
        .connect(server_addr, server_name)
        .map_err(io::Error::other)?;
    connecting.await.map_err(io::Error::other)
}

/// Bind an ephemeral local address in the same family as the server.
fn bind_addr(server_addr: SocketAddr) -> SocketAddr {
    if server_addr.is_ipv6() {
        "[::]:0".parse().expect("valid v6 bind addr")
    } else {
        "0.0.0.0:0".parse().expect("valid v4 bind addr")
    }
}
