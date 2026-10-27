use crate::account::parse_mark;
use crate::account::Account;
use crate::account::Frozen;
use crate::account::Marks;
use crate::account::Overview;
use crate::output::Failure;
use rsx_risk::Position;
use serde_json::json;

fn long_position(symbol_id: u32, qty: i64, cost: i64) -> Position {
    Position {
        user_id: 7,
        symbol_id,
        long_qty: qty,
        long_entry_cost: cost,
        ..Position::default()
    }
}

fn short_position(symbol_id: u32, qty: i64, cost: i64) -> Position {
    Position {
        user_id: 7,
        symbol_id,
        short_qty: qty,
        short_entry_cost: cost,
        ..Position::default()
    }
}

fn account() -> Account {
    Account {
        user_id: 7,
        collateral: 1_000_000,
        positions: vec![long_position(10, 10, 1_000), short_position(11, 5, 500)],
        frozen: vec![Frozen {
            order_id: "0000000000000001000000000000000a".to_string(),
            symbol_id: 10,
            amount: 40,
        }],
    }
}

#[test]
fn parse_mark_reads_symbol_and_raw_price() {
    assert_eq!(parse_mark("10=2000000"), Ok((10, 2_000_000)));
    for text in ["10", "x=5", "10=1.5", "10=", "=5"] {
        assert!(parse_mark(text).is_err(), "{text}");
    }
}

#[test]
fn account_json_carries_net_qty_entry_price_and_upnl_at_the_mark() {
    let marks: Marks = [(10, 120), (11, 90)].into_iter().collect();
    assert_eq!(
        account().to_json(&marks).unwrap(),
        json!({
            "user_id": 7,
            "collateral": 1_000_000,
            "positions": [
                {"symbol_id": 10, "net_qty": 10, "avg_px": 100, "upnl": 200},
                {"symbol_id": 11, "net_qty": -5, "avg_px": 100, "upnl": 50},
            ],
            "frozen": [
                {
                    "order_id": "0000000000000001000000000000000a",
                    "symbol_id": 10,
                    "amount": 40,
                },
            ],
        })
    );
}

#[test]
fn upnl_is_null_for_a_symbol_without_a_mark() {
    let marks: Marks = [(10, 120)].into_iter().collect();
    let value = account().to_json(&marks).unwrap();
    assert_eq!(value["positions"][0]["upnl"], 200);
    assert!(value["positions"][1]["upnl"].is_null());
}

#[test]
fn an_overflowing_upnl_is_a_runtime_failure() {
    let account = Account {
        user_id: 7,
        collateral: 0,
        positions: vec![long_position(10, i64::MAX, 0)],
        frozen: Vec::new(),
    };
    let marks: Marks = [(10, 10)].into_iter().collect();
    let failure = account.to_json(&marks).unwrap_err();
    assert!(matches!(failure, Failure::Runtime(_)), "{failure:?}");
}

#[test]
fn frozen_margin_sums_per_account_and_an_overflow_fails() {
    assert_eq!(account().frozen_total().unwrap(), 40);
    let overflowing = Account {
        user_id: 7,
        collateral: 0,
        positions: Vec::new(),
        frozen: vec![
            Frozen {
                order_id: String::new(),
                symbol_id: 1,
                amount: i64::MAX,
            },
            Frozen {
                order_id: String::new(),
                symbol_id: 1,
                amount: 1,
            },
        ],
    };
    assert!(overflowing.frozen_total().is_err());
}

#[test]
fn overview_json_has_users_insurance_and_liquidations() {
    let overview = Overview {
        accounts: vec![account()],
        insurance: vec![(10, 700, 3), (11, 300, 1)],
        liquidations: vec![json!({"user_id": 9, "timestamp_ns": 5})],
    };
    let value = overview.to_json(&Marks::new()).unwrap();
    assert_eq!(value["users"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["insurance"],
        json!({
            "funds": [
                {"symbol_id": 10, "balance": 700, "version": 3},
                {"symbol_id": 11, "balance": 300, "version": 1},
            ],
            "total": 1000,
        })
    );
    assert_eq!(value["liquidations"][0]["user_id"], 9);
}

#[test]
fn overview_text_lists_each_account_and_the_totals() {
    let overview = Overview {
        accounts: vec![account()],
        insurance: vec![(10, 700, 3)],
        liquidations: Vec::new(),
    };
    let text = overview.to_text().unwrap();
    assert!(text.contains("     7  1000000  40  2"), "{text}");
    assert!(text.contains("1 accounts, insurance fund 700"), "{text}");
}
