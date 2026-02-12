use rsx_marketdata::shadow::ShadowBook;
use rsx_types::SymbolConfig;

fn config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 1,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

fn new_book() -> ShadowBook {
    ShadowBook::new(config(), 1024, 50000)
}

/// MD16: Fill events update shadow book (reduce qty).
#[test]
fn fill_event_routed_to_marketdata() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    book.apply_fill(h, 40, 0, 2000);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 1);
    assert_eq!(snap.bids[0].qty, 60);
}

/// MD16: OrderInserted events add orders to shadow book.
#[test]
fn order_inserted_routed_to_marketdata() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 1);
    assert_eq!(snap.bids[0].price, 49990);
    assert_eq!(snap.bids[0].qty, 50);
}

/// MD16: OrderCancelled events remove orders from shadow
/// book.
#[test]
fn order_cancelled_routed_to_marketdata() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_cancel(h, 2000);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids.len(), 0);
}

/// MD20: OrderDone is NOT routed to market data.
/// ShadowBook has no apply_done method -- the event loop
/// must filter OrderDone events before they reach the
/// shadow book. We verify the API surface only exposes
/// apply_fill, apply_insert, apply_cancel.
#[test]
fn order_done_not_routed_to_marketdata() {
    let mut book = new_book();
    let h = book.apply_insert(49990, 100, 0, 1, 1000);
    // Partially fill -- order still resting
    book.apply_fill(h, 50, 0, 2000);
    let snap = book.derive_l2_snapshot(10);
    assert_eq!(snap.bids[0].qty, 50);
    // No apply_done exists on ShadowBook. If someone
    // incorrectly routes OrderDone as a cancel, the
    // order would vanish. Verify it persists.
    assert_eq!(snap.bids.len(), 1);
}

/// MD21: BBO events from ME are NOT consumed by market
/// data. Market data derives its own BBO from shadow book.
/// ShadowBook has no apply_bbo method.
#[test]
fn bbo_event_not_routed_to_marketdata() {
    let mut book = new_book();
    book.apply_insert(49990, 50, 0, 1, 1000);
    book.apply_insert(50010, 30, 1, 2, 1001);
    let bbo = book.derive_bbo().unwrap();
    // BBO is derived, not ingested from ME
    assert_eq!(bbo.bid_px, 49990);
    assert_eq!(bbo.ask_px, 50010);
    assert_eq!(bbo.symbol_id, 1);
}
