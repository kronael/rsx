use crate::gateway_client::Connection;
use crate::stress::cross_down;
use crate::stress::cross_up;
use crate::stress::send_on_schedule;
use crate::stress::user_for;
use crate::stress::Plan;
use crate::stress::Run;
use crate::stress::Schedule;
use crate::stress_tally::Measure;
use crate::stress_tally::Tracker;
use crate::token::mint_now;
use futures_util::StreamExt;
use rsx_messages::WsFrame;
use rsx_types::client_order::CidMint;
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_tungstenite::accept_async;

fn plan() -> Plan {
    Plan {
        symbol_id: 10,
        lot_size: 1000,
        bid_px: 49_000,
        ask_px: 51_000,
        bid_qty: 5000,
        ask_qty: 6000,
        buy_px: 51_500,
        sell_px: 48_500,
        buy_qty: 3000,
        sell_qty: 4000,
        tif: 1,
    }
}

#[test]
fn orders_alternate_between_buying_the_ask_and_selling_the_bid() {
    let plan = plan();
    let WsFrame::NewOrder {
        side,
        price,
        qty,
        tif,
        ..
    } = plan.order(0, "a".to_string())
    else {
        panic!("not a new order");
    };
    assert_eq!((side, price, qty, tif), (0, 51_500, 3000, 1));
    let WsFrame::NewOrder {
        side,
        price,
        qty,
        client_order_id,
        ..
    } = plan.order(1, "b".to_string())
    else {
        panic!("not a new order");
    };
    assert_eq!((side, price, qty), (1, 48_500, 4000));
    assert_eq!(client_order_id, "b");
}

#[test]
fn connections_cycle_over_the_users_and_skip_the_maker() {
    let users: Vec<u32> = (0..4).map(|index| user_for(index, 3)).collect();
    assert_eq!(users, [1, 2, 3, 1]);
    assert_eq!(user_for(98, 200), 100);
    assert_eq!(user_for(97, 200), 98);
}

async fn serve_and_discard() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(discard_frames(listener));
    url
}

async fn discard_frames(listener: TcpListener) {
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = accept_async(stream).await.unwrap();
    while socket.next().await.is_some() {}
}

#[tokio::test]
async fn a_client_behind_its_schedule_sends_the_backlog_and_counts_the_lateness() {
    let url = serve_and_discard().await;
    let token = mint_now(7, 60, Some("test-secret-padded-to-32-bytes-minlen!"))
        .unwrap()
        .token;
    let now = Instant::now();
    let mut run = Run {
        connection: Connection::open(&url, &token).await.unwrap(),
        tracker: Tracker::new(Measure::Accepted),
        plan: plan(),
        cids: Rc::new(RefCell::new(CidMint::new())),
        stop: Rc::new(Cell::new(false)),
        timeout: Duration::from_millis(50),
        sent: 0,
    };
    let schedule = Schedule {
        first: now - Duration::from_millis(500),
        interval: Duration::from_millis(10),
        end: now + Duration::from_millis(100),
    };
    send_on_schedule(&mut run, &schedule).await.unwrap();
    run.tracker.finish();
    let tally = &mut run.tracker.tally;
    assert_eq!(tally.submitted, 60);
    assert_eq!(tally.timeouts, 60);
    let lag_us = tally.send_lag_us.summary()["max"].as_u64().unwrap();
    assert!(lag_us >= 450_000, "{lag_us}");
}

#[test]
fn limit_prices_go_past_the_book_by_the_basis_points_on_a_tick() {
    assert_eq!(cross_up(51_000, 100, 1), 51_510);
    assert_eq!(cross_down(49_000, 100, 1), 48_510);
    assert_eq!(cross_up(51_000, 100, 25), 51_525);
    assert_eq!(cross_down(49_000, 100, 25), 48_500);
    assert_eq!(cross_up(51_000, 0, 25), 51_000);
    assert_eq!(cross_down(49_000, 0, 25), 49_000);
}

#[test]
fn a_sell_never_goes_below_one_tick() {
    assert_eq!(cross_down(3, 9_999, 1), 1);
    assert_eq!(cross_down(3, 20_000, 1), 1);
}
