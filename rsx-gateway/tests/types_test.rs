use rsx_gateway::types::*;

#[test]
fn risk_new_order_alignment() {
    assert_eq!(
        std::mem::align_of::<RiskNewOrder>(),
        64,
    );
    assert_eq!(
        std::mem::size_of::<RiskNewOrder>() % 64,
        0,
    );
}

#[test]
fn risk_cancel_order_alignment() {
    assert_eq!(
        std::mem::align_of::<RiskCancelOrder>(),
        64,
    );
    assert_eq!(
        std::mem::size_of::<RiskCancelOrder>() % 64,
        0,
    );
}

#[test]
fn risk_order_update_alignment() {
    assert_eq!(
        std::mem::align_of::<RiskOrderUpdate>(),
        64,
    );
    assert_eq!(
        std::mem::size_of::<RiskOrderUpdate>() % 64,
        0,
    );
}

#[test]
fn order_fill_alignment() {
    assert_eq!(
        std::mem::align_of::<OrderFill>(),
        64,
    );
    assert_eq!(
        std::mem::size_of::<OrderFill>() % 64,
        0,
    );
}

#[test]
fn stream_error_alignment() {
    assert_eq!(
        std::mem::align_of::<StreamError>(),
        64,
    );
    assert_eq!(
        std::mem::size_of::<StreamError>() % 64,
        0,
    );
}

#[test]
fn risk_new_order_fields() {
    let order = RiskNewOrder {
        order_id: [1; 16],
        client_order_id: [2; 20],
        user_id: 42,
        symbol_id: 1,
        side: 0,
        tif: 0,
        reduce_only: 0,
        is_liquidation: 0,
        _pad: [0; 4],
        price: 5000000,
        qty: 1000,
        timestamp_ns: 123456789,
    };
    assert_eq!(order.user_id, 42);
    assert_eq!(order.price, 5000000);
}

#[test]
fn order_fill_fee_signed() {
    let fill = OrderFill {
        taker_order_id: [0; 16],
        maker_order_id: [0; 16],
        taker_user_id: 1,
        maker_user_id: 2,
        price: 100,
        qty: 10,
        taker_side: 0,
        _pad: [0; 7],
        timestamp_ns: 0,
        taker_fee: 5,
        maker_fee: -1,
    };
    assert!(fill.taker_fee > 0);
    assert!(fill.maker_fee < 0);
}
