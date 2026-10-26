use crate::probe_qty;
use crate::SymbolRow;

const ONE_USD: i64 = 10_000_000_000;

fn pengu(min_notional: Option<i64>) -> SymbolRow {
    SymbolRow {
        id: 10,
        lot_size: 100_000,
        price_decimals: 6,
        qty_decimals: 4,
        min_notional,
    }
}

#[test]
fn catalog_minimum_prefers_the_row_then_one_usd() {
    assert_eq!(
        pengu(Some(30_000_000_000)).minimum_notional(),
        30_000_000_000
    );
    assert_eq!(pengu(None).minimum_notional(), ONE_USD);
    assert_eq!(pengu(Some(0)).minimum_notional(), ONE_USD);
}

#[test]
fn probe_order_reaches_the_catalog_minimum_at_its_price() {
    for cross_px in [29_000_i64, 30_000, 31_007, 6_000_000] {
        let (lot_size, min_notional, qty) = probe_qty(&pengu(None), None, None, cross_px);
        assert_eq!(lot_size, 100_000);
        assert_eq!(min_notional, ONE_USD);
        assert_eq!(qty % lot_size, 0);
        assert!(i128::from(cross_px) * i128::from(qty) >= i128::from(min_notional));
    }
}

#[test]
fn probe_flags_override_the_catalog() {
    let (lot_size, min_notional, qty) = probe_qty(
        &pengu(Some(ONE_USD)),
        Some(1_000),
        Some(50_000_000_000),
        30_000,
    );
    assert_eq!(lot_size, 1_000);
    assert_eq!(min_notional, 50_000_000_000);
    assert!(i128::from(30_000) * i128::from(qty) >= i128::from(50_000_000_000_i64));
    assert_eq!(qty % 1_000, 0);
}

#[test]
fn a_catalog_above_one_usd_raises_the_order() {
    let at_one_usd = probe_qty(&pengu(None), None, None, 30_000).2;
    let at_five_usd = probe_qty(&pengu(Some(5 * ONE_USD)), None, None, 30_000).2;
    assert!(at_five_usd > at_one_usd);
}
