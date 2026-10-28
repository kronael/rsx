//! A record fetched from risk's replication server after a FAULTED or
//! RECONNECT takes the ring and the `route_*` functions a live record takes.

mod common;

use common::cid;
use common::done;
use common::fields;
use common::fill;
use common::ids;
use common::order;
use common::order_hex;
use common::replay::serve_wal;
use common::replay::serve_wal_corrupt_at;
use common::replay::unreachable_server;
use common::replay::WalServer;
use common::route_event;
use common::run;
use common::Gateway;
use common::Options;
use rsx_gateway::cast_recv::replay_gap;
use std::sync::atomic::AtomicBool;

/// Replay `server`'s WAL from the start and route what lands on the ring,
/// as the monoio thread does. Returns the tip the replay reports.
fn replay_and_route(gateway: &Gateway, server: &WalServer, skip_to: u64) -> u64 {
    let (mut producer, mut consumer) = rtrb::RingBuffer::new(64);
    let shutdown = AtomicBool::new(false);
    let tip = replay_gap(
        &server.addr.to_string(),
        server.tls.clone(),
        server.tip_file.clone(),
        0,
        skip_to,
        &shutdown,
        &mut producer,
    );
    while let Ok(event) = consumer.pop() {
        route_event(&gateway.state, event);
    }
    tip
}

// replayed fill -> taker and maker each get the frame a live fill produces, once
#[test]
fn replayed_fill_reaches_the_client_like_a_live_one() {
    run(Options::default(), |gateway| async move {
        let mut taker = gateway.connect(7).await;
        let mut maker = gateway.connect(8).await;
        let taker_order = gateway.place(&mut taker, &order(&cid(1))).await;
        let maker_order = gateway.place(&mut maker, &order(&cid(2))).await;
        let record = fill(ids(&taker_order), ids(&maker_order), 1000, 10);

        gateway.risk.inject(record).await;
        let live_to_taker = taker.recv().await;
        let live_to_maker = maker.recv().await;

        let server = serve_wal(vec![record]);
        assert_eq!(replay_and_route(&gateway, &server, 0), server.last_seq);

        // The last field is the gateway's own egress stamp.
        let live = fields(&live_to_taker, "F");
        let replayed = fields(&taker.recv().await, "F");
        assert_eq!(replayed[..10], live[..10]);
        assert_eq!(fields(&maker.recv().await, "F")[..10], live[..10]);
        assert_eq!(fields(&live_to_maker, "F")[..10], live[..10]);
        assert_eq!(taker.settle().await, Vec::<String>::new());
        assert_eq!(maker.settle().await, Vec::<String>::new());
    });
}

// replayed ORDER_DONE -> U for the owner and the pending entry is gone
#[test]
fn replayed_completion_reaches_the_owner_and_removes_pending() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        assert_eq!(gateway.state.borrow().pending.len(), 1);

        let server = serve_wal(vec![done(ids(&placed), 10, 0, 0)]);
        replay_and_route(&gateway, &server, 0);

        let update = fields(&client.recv().await, "U");
        assert_eq!(update[0], order_hex(&placed));
        assert_eq!(update[1], 0);
        assert_eq!(update[2], 10);
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}

// drain fails after two records -> the tip resumes past them, none repeats
#[test]
fn failed_drain_resumes_past_the_records_it_pushed() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let placed = gateway.place(&mut client, &order(&cid(1))).await;
        let records = (0..4).map(|_| done(ids(&placed), 10, 0, 0)).collect();
        let server = serve_wal_corrupt_at(records, 2);
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(64);
        let shutdown = AtomicBool::new(false);

        let tip = replay_gap(
            &server.addr.to_string(),
            server.tls.clone(),
            server.tip_file.clone(),
            0,
            0,
            &shutdown,
            &mut producer,
        );

        let mut pushed = 0;
        while consumer.pop().is_ok() {
            pushed += 1;
        }
        assert_eq!(pushed, 2);
        assert_eq!(tip, 2);
    });
}

// replay server unreachable -> the gap is skipped, nothing is routed, no panic
#[test]
fn failed_drain_skips_the_gap() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let unreachable = unreachable_server();

        assert_eq!(replay_and_route(&gateway, &unreachable, 9), 9);
        assert_eq!(client.settle().await, Vec::<String>::new());
    });
}
