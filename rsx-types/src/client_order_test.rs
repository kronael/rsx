use crate::client_order::base36;
use crate::client_order::min_order_qty;
use crate::client_order::one_usd_notional;
use crate::client_order::CidMint;
use crate::client_order::SESSION_CHARS;
use std::collections::HashSet;

const ONE_USD: i64 = 10_000_000_000;

fn assert_cid_shape(cid: &str) {
    assert!(
        (1..=20).contains(&cid.len()),
        "cid {cid:?} is not 1-20 chars"
    );
    assert!(
        cid.bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase()),
        "cid {cid:?} has chars outside [0-9a-z]"
    );
}

#[test]
fn base36_renders_without_leading_zeros() {
    assert_eq!(base36(0), "0");
    assert_eq!(base36(9), "9");
    assert_eq!(base36(10), "a");
    assert_eq!(base36(35), "z");
    assert_eq!(base36(36), "10");
    assert_eq!(base36(36 * 36), "100");
    assert_eq!(base36(u64::MAX).len(), 13);
}

#[test]
fn cids_never_collide_over_n_orders() {
    let mut mint = CidMint::new();
    let mut seen = HashSet::new();
    for _ in 0..200_000 {
        let cid = mint.next();
        assert_cid_shape(&cid);
        assert!(seen.insert(cid.clone()), "cid {cid} repeated");
    }
}

#[test]
fn counters_one_ten_hundred_stay_distinct() {
    // A cid right-padded with "0" makes counters 1, 10 and 100 one string.
    let mut mint = CidMint::with_seed(12345);
    let cids: Vec<String> = (0..1296).map(|_| mint.next()).collect();
    let distinct: HashSet<&String> = cids.iter().collect();
    assert_eq!(distinct.len(), cids.len());
    assert!(cids[0].ends_with('1'));
    assert!(cids[35].ends_with("10"));
    assert!(cids[1295].ends_with("100"));
}

#[test]
fn cids_are_session_then_unpadded_counter() {
    let mut mint = CidMint::with_seed(42);
    assert_eq!(mint.session().len(), SESSION_CHARS);
    let session = mint.session().to_string();
    assert_eq!(mint.next(), format!("{session}1"));
    assert_eq!(mint.next(), format!("{session}2"));
}

#[test]
fn two_simulated_restarts_share_no_cid() {
    // Each run draws its own session, so the same counter range of two runs
    // inside the gateway's one-hour window repeats no cid.
    let mut seen = HashSet::new();
    for _ in 0..200 {
        let mut mint = CidMint::new();
        for _ in 0..200 {
            let cid = mint.next();
            assert!(seen.insert(cid.clone()), "cid {cid} repeated across runs");
        }
    }
}

#[test]
fn sessions_span_forty_random_bits() {
    assert!(36_u64.pow(SESSION_CHARS as u32) >= 1 << 40);
    let sessions: HashSet<String> = (0..1000_u64)
        .map(|seed| {
            CidMint::with_seed(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15))
                .session()
                .to_string()
        })
        .collect();
    assert_eq!(sessions.len(), 1000);
}

#[test]
fn min_order_qty_is_the_smallest_lot_multiple_at_the_floor() {
    for (price, lot) in [
        (30_000_i64, 100_000_i64),
        (6_000_000, 1_000),
        (31, 100_000),
        (1, 1),
        (999_999, 7),
    ] {
        let qty = min_order_qty(ONE_USD, price, lot);
        assert_eq!(qty % lot, 0);
        assert!(qty >= lot);
        assert!(i128::from(price) * i128::from(qty) >= i128::from(ONE_USD));
        if qty > lot {
            assert!(i128::from(price) * i128::from(qty - lot) < i128::from(ONE_USD));
        }
    }
}

#[test]
fn min_order_qty_at_the_floor_is_exact() {
    assert_eq!(min_order_qty(1000, 5, 10), 200);
    assert_eq!(min_order_qty(1001, 5, 10), 210);
    assert_eq!(min_order_qty(0, 5, 10), 10);
    assert_eq!(min_order_qty(100, 1_000_000, 50), 50);
}

#[test]
fn min_order_qty_saturates_instead_of_overflowing() {
    let qty = min_order_qty(i64::MAX, 1, 3);
    assert!(qty > 0 && qty % 3 == 0);
}

#[test]
fn one_usd_is_ten_to_the_decimals() {
    assert_eq!(one_usd_notional(6, 4), ONE_USD);
    assert_eq!(one_usd_notional(2, 8), ONE_USD);
    assert_eq!(one_usd_notional(4, 6), ONE_USD);
    assert_eq!(one_usd_notional(30, 30), i64::MAX);
}
