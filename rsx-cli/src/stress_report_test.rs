use crate::stress_report::build;
use crate::stress_report::Settings;
use crate::stress_tally::Measure;
use crate::stress_tally::Tally;
use crate::stress_tally::Tracker;
use rsx_messages::WsFrame;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::time::Instant;

const OID: &str = "0190000000007000800000000000000a";
const STAMPS: [u64; 5] = [1000, 2000, 4000, 7000, 11_000];

fn settings(measure: Measure) -> Settings {
    Settings {
        mode: "load",
        measure,
        gateway: "ws://127.0.0.1:8088".to_string(),
        symbols: vec![10],
        users: 1,
        connections: 1,
        target_rate: 100,
        duration_s: 1,
        count: 0,
        warmup: 0,
        timeout_ms: 1000,
        is_crossing: true,
    }
}

fn update(order_id: &str, status: u8, reason: u8, stamps: [u64; 5]) -> WsFrame {
    WsFrame::OrderUpdate {
        order_id: order_id.to_string(),
        status,
        filled_qty: 1,
        remaining_qty: 0,
        reason,
        gw_in_ns: stamps[0],
        risk_in_ns: stamps[1],
        me_in_ns: stamps[2],
        match_done_ns: stamps[3],
        gw_out_ns: stamps[4],
    }
}

fn fill(order_id: &str) -> WsFrame {
    WsFrame::Fill {
        taker_order_id: order_id.to_string(),
        maker_order_id: order_id.to_string(),
        price: 100,
        qty: 1,
        timestamp_ns: 5000,
        fee: 0,
        gw_in_ns: STAMPS[0],
        risk_in_ns: STAMPS[1],
        me_in_ns: STAMPS[2],
        match_done_ns: STAMPS[3],
        gw_out_ns: STAMPS[4],
    }
}

fn filled_tally(count: usize) -> Tally {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    for index in 0..count {
        let oid = format!("{index:032x}");
        tracker.record_sent(base, base);
        tracker.accept(&fill(&oid), base + Duration::from_millis(1));
        tracker.accept(&update(&oid, 0, 0, STAMPS), base + Duration::from_millis(2));
    }
    tracker.tally
}

fn by_symbol(tally: Tally) -> BTreeMap<u32, Tally> {
    BTreeMap::from([(10, tally)])
}

#[test]
fn a_sound_run_passes_and_reports_every_contract_key() {
    let report = build(
        &settings(Measure::Accepted),
        by_symbol(filled_tally(4)),
        Duration::from_secs(2),
        false,
    );
    assert_eq!(report.failures, Vec::<String>::new());
    let json = &report.json;
    assert_eq!(json["status"], "passed");
    assert_eq!(json["mode"], "load");
    for key in [
        "offered",
        "submitted",
        "send_errors",
        "accepted",
        "rejected",
        "timeouts",
        "errors",
        "completed",
        "fills",
        "filled_orders",
    ] {
        assert!(json[key].is_u64(), "{key}");
    }
    assert_eq!(json["offered"], 4);
    assert_eq!(json["accepted"], 4);
    assert_eq!(json["elapsed_ms"], 2000);
    for key in ["n", "min", "p50", "p95", "p99", "p99_9", "max"] {
        assert!(!json["latency_us"][key].is_null(), "{key}");
    }
    for leg in [
        "internal",
        "gateway_to_risk",
        "risk_to_me",
        "matching",
        "match_done_to_gateway",
    ] {
        assert_eq!(json["server_latency_ns"][leg]["n"], 4, "{leg}");
    }
    assert_eq!(json["per_symbol"]["10"]["accepted"], 4);
    assert_eq!(json["config"]["measures"], "accepted");
}

#[test]
fn a_run_with_no_answers_fails() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.finish();
    let report = build(
        &settings(Measure::Accepted),
        by_symbol(tracker.tally),
        Duration::from_secs(1),
        false,
    );
    assert_eq!(report.json["status"], "failed");
    assert!(report
        .failures
        .contains(&"zero accepted latency samples".to_string()));
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.starts_with("answered orders below 95%")));
    assert!(report
        .failures
        .contains(&"symbol 10 completed no work".to_string()));
}

#[test]
fn a_crossing_order_that_did_not_fill_fails() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.accept(&update(OID, 2, 0, STAMPS), base);
    let report = build(
        &settings(Measure::Accepted),
        by_symbol(tracker.tally),
        Duration::from_secs(1),
        false,
    );
    assert!(report
        .failures
        .contains(&"accepted crossing orders did not all fill".to_string()));
}

#[test]
fn unsound_server_timestamps_fail() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.accept(&update(OID, 0, 0, [0; 5]), base);
    let report = build(
        &settings(Measure::Accepted),
        by_symbol(tracker.tally),
        Duration::from_secs(1),
        false,
    );
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.starts_with("invalid server timestamps")));
    assert!(report
        .failures
        .contains(&"internal samples do not equal 1".to_string()));
}

#[test]
fn rejects_are_counted_by_reason_and_do_not_fail_a_load() {
    let mut tally = filled_tally(3);
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Accepted);
    tracker.record_sent(base, base);
    tracker.accept(&update(OID, 3, 14, [0; 5]), base);
    tally.merge(&tracker.tally);
    let report = build(
        &settings(Measure::Accepted),
        by_symbol(tally),
        Duration::from_secs(1),
        false,
    );
    assert_eq!(report.failures, Vec::<String>::new());
    assert_eq!(report.json["rejected"], 1);
    assert_eq!(report.json["rejected_by_reason"]["ORDER_TOO_SMALL"], 1);
}

#[test]
fn a_reject_run_fails_when_an_order_is_accepted() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Measure::Rejected);
    tracker.record_sent(base, base);
    tracker.record_sent(base, base);
    tracker.accept(
        &WsFrame::Error {
            code: 1007,
            message: "unknown symbol".to_string(),
        },
        base,
    );
    tracker.accept(&update(OID, 1, 0, [0; 5]), base);
    let mut crossing = settings(Measure::Rejected);
    crossing.is_crossing = false;
    let report = build(
        &crossing,
        by_symbol(tracker.tally),
        Duration::from_secs(1),
        false,
    );
    assert_eq!(report.failures, vec!["1 orders were accepted".to_string()]);
    assert_eq!(report.json["rejected_by_reason"]["E1007"], 1);
}

#[test]
fn an_interrupted_run_says_so() {
    let report = build(
        &settings(Measure::Accepted),
        by_symbol(filled_tally(1)),
        Duration::from_secs(1),
        true,
    );
    assert_eq!(report.json["interrupted"], true);
}
