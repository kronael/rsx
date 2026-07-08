//! Loopback proof for the QUIC client transport.
//!
//! Stands up a self-signed `quinn` server on `127.0.0.1:0`, connects a
//! real `QuicConn` trusting the test cert, submits one order, and polls
//! `poll_event` until it observes `Connected` plus the `Fill` the server
//! echoed back. This exercises both directions over an actual QUIC
//! bidirectional stream — order out, event in — through the synchronous
//! `GatewayConn` API the UI uses.

use rsx_tui::conn::GatewayConn;
use rsx_tui::conn::GwEvent;
use rsx_tui::conn::OrderReq;
use rsx_tui::conn::Side;
use rsx_tui::conn::Tif;
use rsx_tui::quic::mint_jwt;
use rsx_tui::quic::roots;
use rsx_tui::quic::QuicConn;
use rsx_tui::quic::Session;
use rsx_tui::wire;
use rsx_tui::MockConn;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use quinn::Endpoint;
use quinn::ServerConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivatePkcs8KeyDer;

/// Generate a self-signed cert for `localhost` and its DER cert + key.
fn self_signed() -> (CertificateDer<'static>, PrivatePkcs8KeyDer<'static>) {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("generate self-signed cert");
    let der = cert.cert.der().clone();
    let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
    (der, key)
}

/// Accept one connection + one bi stream, read the auth first-frame then
/// one order, report the order (with the user from the hello), and echo a
/// `Fill` + a `Latency` carrying the order's `cid`. Then block reading
/// until the client disconnects so the connection stays alive long enough
/// for the client to receive the events.
async fn run_server(endpoint: Endpoint, got: mpsc::Sender<(u32, OrderReq)>) {
    let Some(incoming) = endpoint.accept().await else {
        return;
    };
    let conn = match incoming.await {
        Ok(conn) => conn,
        Err(_) => return,
    };
    let (mut send, mut recv) = match conn.accept_bi().await {
        Ok(stream) => stream,
        Err(_) => return,
    };
    // Auth first-frame: the client sends WireHello (JWT + user) before any
    // order. Reading it keeps the stream framed and proves identity is
    // carried in-band.
    let hello = match wire::read_hello(&mut recv).await {
        Ok(hello) => hello,
        Err(_) => return,
    };
    let incoming = match wire::read_order(&mut recv).await {
        Ok(incoming) => incoming,
        Err(_) => return,
    };
    let _ = got.send((hello.user, incoming.order));
    let fill = GwEvent::Fill {
        oid: 7,
        px: incoming.order.price,
        qty: incoming.order.qty,
        side: incoming.order.side,
    };
    if wire::write_event(&mut send, &fill).await.is_err() {
        return;
    }
    // Timing report: the gateway knows internal (casting) + engine time;
    // net is `None` for the client to fill from the measured round-trip.
    // The order's `cid` is echoed so the client pairs the sample by id.
    // Small stamps so the real loopback RTT always exceeds them and the
    // derived net is deterministically `Some` (not underflow → None).
    let lat = GwEvent::Latency {
        cid: incoming.cid,
        net_ns: None,
        internal_ns: 500,
        engine_ns: 100,
    };
    if wire::write_event(&mut send, &lat).await.is_err() {
        return;
    }
    // Hold the connection open until the client closes (returns Err).
    let _ = wire::read_order(&mut recv).await;
}

#[test]
fn quic_loopback_roundtrips_order_and_event() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build test runtime");

    let (der, key) = self_signed();
    let srv_cfg =
        ServerConfig::with_single_cert(vec![der.clone()], key.into()).expect("server config");
    let endpoint = rt.block_on(async {
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("addr");
        Endpoint::server(srv_cfg, addr).expect("server endpoint")
    });
    let addr = endpoint.local_addr().expect("bound addr");

    let (order_tx, order_rx) = mpsc::channel::<(u32, OrderReq)>();
    rt.spawn(run_server(endpoint, order_tx));

    let store = roots([der]).expect("root store");
    let session = Session {
        symbol_id: 42,
        user: 1,
        jwt: mint_jwt(1, "test-secret"),
    };
    let mut conn = QuicConn::connect(addr, "localhost", store, session).expect("connect QuicConn");

    let want = OrderReq {
        side: Side::Buy,
        price: 10_001,
        qty: 5,
        tif: Tif::Ioc,
    };
    conn.submit(want).expect("submit order");

    let mut saw_connected = false;
    let mut saw_fill: Option<GwEvent> = None;
    let mut saw_lat: Option<GwEvent> = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && (!saw_connected || saw_fill.is_none() || saw_lat.is_none()) {
        while let Some(ev) = conn.poll_event() {
            match ev {
                GwEvent::Connected => saw_connected = true,
                GwEvent::Fill { .. } => saw_fill = Some(ev),
                GwEvent::Latency { .. } => saw_lat = Some(ev),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    assert!(saw_connected, "client never observed Connected");
    let fill = saw_fill.expect("client never observed Fill");
    assert_eq!(
        fill,
        GwEvent::Fill {
            oid: 7,
            px: 10_001,
            qty: 5,
            side: Side::Buy
        },
    );

    // The client filled the net leg from its measured round-trip; the
    // server-stamped internal + engine components pass through unchanged.
    match saw_lat.expect("client never observed Latency") {
        GwEvent::Latency {
            net_ns,
            internal_ns,
            engine_ns,
            ..
        } => {
            assert_eq!(internal_ns, 500);
            assert_eq!(engine_ns, 100);
            let net = net_ns.expect("client measured the net leg");
            assert!(net > 0, "net (client↔gateway) leg is a real measurement");
        }
        other => panic!("expected Latency, got {other:?}"),
    }

    let (got_user, got_order) = order_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server received the order");
    assert_eq!(
        got_user, 1,
        "server saw the session user id from the auth first-frame",
    );
    assert_eq!(
        got_order, want,
        "server saw the exact order the client submitted",
    );
}

/// The in-memory transport the UI is built and demoed against still
/// round-trips — the QUIC work is additive, not a replacement.
#[test]
fn mock_conn_still_round_trips() {
    let mut conn = MockConn::new();
    conn.push_events([GwEvent::Connected, GwEvent::Accepted { oid: 1 }]);

    let order = OrderReq {
        side: Side::Sell,
        price: 9_999,
        qty: 3,
        tif: Tif::Gtc,
    };
    conn.submit(order).expect("mock submit");
    assert_eq!(conn.submitted, vec![order]);

    assert_eq!(conn.poll_event(), Some(GwEvent::Connected));
    assert_eq!(conn.poll_event(), Some(GwEvent::Accepted { oid: 1 }));
    assert_eq!(conn.poll_event(), None);
}
