use crate::order::cancel_target;
use crate::order::check_cid;
use crate::order::parse_batch;
use crate::order::resolve;
use crate::order::CancelTarget;
use crate::order::OrderInput;
use crate::order::PriceInput;
use crate::order::QtyInput;
use crate::order::Side;
use crate::order::Tif;
use crate::output::Failure;
use crate::symbols::Symbol;
use rsx_messages::WsFrame;
use rsx_types::client_order::CidMint;
use std::collections::HashMap;
use std::collections::HashSet;

const OID: &str = "0190000000007000800000000000000A";

fn symbols() -> Vec<Symbol> {
    vec![Symbol {
        symbol_id: 10,
        tick_size: 1,
        lot_size: 1000,
        price_decimals: 2,
        qty_decimals: 3,
    }]
}

fn input(price: PriceInput, qty: QtyInput) -> OrderInput {
    OrderInput {
        user_id: 1,
        symbol_id: 10,
        side: Side::Sell,
        tif: Tif::Ioc,
        price,
        qty,
        reduce_only: true,
        post_only: false,
        cid: None,
    }
}

fn new_order(frame: &WsFrame) -> (i64, i64, String, u8, u8) {
    match frame {
        WsFrame::NewOrder {
            side,
            price,
            qty,
            client_order_id,
            tif,
            ..
        } => (*price, *qty, client_order_id.clone(), *side, *tif),
        other => panic!("not a new order: {other:?}"),
    }
}

#[test]
fn test_human_price_and_qty_convert_with_the_symbol_decimals() {
    let order = input(
        PriceInput::Human("12.34".to_string()),
        QtyInput::Human("2".to_string()),
    );
    let resolved = resolve(&order, &symbols(), &HashMap::new(), &mut CidMint::new()).unwrap();
    let (price, qty, _, side, tif) = new_order(&resolved.frame);
    assert_eq!((price, qty, side, tif), (1234, 2000, 1, 1));
}

#[test]
fn test_raw_price_and_qty_pass_through_without_a_catalog() {
    let order = input(PriceInput::Raw(7), QtyInput::Raw(3));
    let resolved = resolve(&order, &[], &HashMap::new(), &mut CidMint::new()).unwrap();
    let (price, qty, _, _, _) = new_order(&resolved.frame);
    assert_eq!((price, qty), (7, 3));
}

#[test]
fn test_min_size_is_the_smallest_lot_multiple_reaching_one_usd() {
    let order = input(PriceInput::Raw(30), QtyInput::Minimum);
    let resolved = resolve(&order, &symbols(), &HashMap::new(), &mut CidMint::new()).unwrap();
    let (_, qty, _, _, _) = new_order(&resolved.frame);
    assert_eq!(qty, 4000);
    assert!(30 * qty >= 100_000);
    assert!(30 * (qty - 1000) < 100_000);
}

#[test]
fn test_min_size_honours_the_notional_override() {
    let order = input(PriceInput::Raw(30_000), QtyInput::Minimum);
    let resolved = resolve(
        &order,
        &symbols(),
        &HashMap::from([(10, 300_000_000)]),
        &mut CidMint::new(),
    )
    .unwrap();
    let (_, qty, _, _, _) = new_order(&resolved.frame);
    assert_eq!(qty, 10_000);
}

#[test]
fn test_unrepresentable_human_value_is_misuse() {
    let order = input(
        PriceInput::Human("1.234".to_string()),
        QtyInput::Human("1".to_string()),
    );
    let failure = resolve(&order, &symbols(), &HashMap::new(), &mut CidMint::new())
        .err()
        .unwrap();
    assert!(matches!(failure, Failure::Misuse(_)), "{failure:?}");
}

#[test]
fn test_unknown_symbol_for_a_human_order_is_misuse() {
    let mut order = input(
        PriceInput::Human("1".to_string()),
        QtyInput::Human("1".to_string()),
    );
    order.symbol_id = 99;
    let failure = resolve(&order, &symbols(), &HashMap::new(), &mut CidMint::new())
        .err()
        .unwrap();
    assert!(matches!(failure, Failure::Misuse(_)), "{failure:?}");
}

#[test]
fn test_minted_cids_are_unique_and_valid() {
    let mut cids = CidMint::new();
    let mut seen = HashSet::new();
    for _ in 0..1000 {
        let order = input(PriceInput::Raw(1), QtyInput::Raw(1));
        let resolved = resolve(&order, &[], &HashMap::new(), &mut cids).unwrap();
        assert!(check_cid(&resolved.cid).is_ok(), "{}", resolved.cid);
        assert!(seen.insert(resolved.cid));
    }
}

#[test]
fn test_two_mints_do_not_share_cids() {
    let first = CidMint::new().next();
    let second = CidMint::new().next();
    assert_ne!(first, second);
}

#[test]
fn test_a_given_cid_is_sent_as_is_and_a_bad_one_is_misuse() {
    let mut order = input(PriceInput::Raw(1), QtyInput::Raw(1));
    order.cid = Some("my-cid_1".to_string());
    let resolved = resolve(&order, &[], &HashMap::new(), &mut CidMint::new()).unwrap();
    assert_eq!(new_order(&resolved.frame).2, "my-cid_1");
    for bad in ["", "has space", "ünï", "123456789012345678901", "a\"b"] {
        order.cid = Some(bad.to_string());
        let failure = resolve(&order, &[], &HashMap::new(), &mut CidMint::new())
            .err()
            .unwrap();
        assert!(matches!(failure, Failure::Misuse(_)), "{bad:?}");
    }
}

#[test]
fn test_cancel_target_is_an_oid_at_32_hex_characters_else_a_cid() {
    assert!(matches!(
        cancel_target(OID).unwrap(),
        CancelTarget::Oid(oid) if oid == OID.to_ascii_lowercase()
    ));
    assert!(matches!(
        cancel_target("abc123").unwrap(),
        CancelTarget::Cid(cid) if cid == "abc123"
    ));
    assert!(cancel_target("not a key!").is_err());
}

#[test]
fn test_batch_line_maps_every_field() {
    let inputs = parse_batch(
        r#"{"user_id":3,"symbol_id":10,"side":"buy","price":"1.5","qty":2,"tif":"fok","reduce_only":true,"post_only":false,"cid":"c1"}
{"user_id":4,"symbol_id":10,"side":"sell","px":150,"min_size":true}

{"user_id":5,"symbol_id":10,"side":"sell","price":1.25,"qty_raw":7}
"#,
    )
    .unwrap();
    assert_eq!(
        inputs,
        [
            OrderInput {
                user_id: 3,
                symbol_id: 10,
                side: Side::Buy,
                tif: Tif::Fok,
                price: PriceInput::Human("1.5".to_string()),
                qty: QtyInput::Human("2".to_string()),
                reduce_only: true,
                post_only: false,
                cid: Some("c1".to_string()),
            },
            OrderInput {
                user_id: 4,
                symbol_id: 10,
                side: Side::Sell,
                tif: Tif::Gtc,
                price: PriceInput::Raw(150),
                qty: QtyInput::Minimum,
                reduce_only: false,
                post_only: false,
                cid: None,
            },
            OrderInput {
                user_id: 5,
                symbol_id: 10,
                side: Side::Sell,
                tif: Tif::Gtc,
                price: PriceInput::Human("1.25".to_string()),
                qty: QtyInput::Raw(7),
                reduce_only: false,
                post_only: false,
                cid: None,
            },
        ]
    );
}

#[test]
fn test_bad_batch_line_is_misuse_naming_its_number() {
    let cases = [
        (
            r#"{"user_id":1,"symbol_id":1,"side":"buy","qty_raw":1}"#,
            "line 1",
        ),
        (
            r#"{"user_id":1,"symbol_id":1,"side":"buy","px":1}"#,
            "line 1",
        ),
        (
            r#"{"user_id":1,"symbol_id":1,"side":"buy","px":1,"price":"1","qty_raw":1}"#,
            "line 1",
        ),
        (
            r#"{"user_id":1,"symbol_id":1,"side":"buy","px":1,"qty":"1","qty_raw":1}"#,
            "line 1",
        ),
        (
            r#"{"user_id":1,"symbol_id":1,"side":"hold","px":1,"qty_raw":1}"#,
            "line 1",
        ),
        (
            r#"{"user_id":1,"symbol_id":1,"side":"buy","px":1,"qty_raw":1,"typo":1}"#,
            "line 1",
        ),
        ("not json", "line 1"),
    ];
    for (line, expected) in cases {
        let failure = parse_batch(line).unwrap_err();
        assert!(matches!(failure, Failure::Misuse(_)), "{line}");
        assert!(failure.message().starts_with(expected), "{line}");
    }
    let good = r#"{"user_id":1,"symbol_id":1,"side":"buy","px":1,"qty_raw":1}"#;
    let failure = parse_batch(&format!("{good}\n\n{{}}\n")).unwrap_err();
    assert!(
        failure.message().starts_with("line 3"),
        "{}",
        failure.message()
    );
}

#[test]
fn test_empty_batch_is_misuse() {
    assert!(matches!(
        parse_batch("\n \n").unwrap_err(),
        Failure::Misuse(_)
    ));
}
