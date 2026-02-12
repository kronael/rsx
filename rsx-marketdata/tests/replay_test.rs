use rsx_types::Price;
use rsx_types::Qty;
use rsx_dxs::FillRecord;
use rsx_dxs::OrderCancelledRecord;
use rsx_dxs::OrderInsertedRecord;
use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;

fn base_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: 0,
        price_decimals: 0,
        qty_decimals: 0,
        tick_size: 1,
        lot_size: 1,
    }
}

fn new_state() -> MarketDataState {
    MarketDataState::new(4, base_config(), 256, 100)
}

/// Run test body on a thread with 16MB stack
/// (Orderbook event_buf is ~1MB, default 8MB
/// stack overflows in debug builds).
fn big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn replay_events_apply_to_shadow_book() {
    big_stack(|| {
        let mut state = new_state();
        let insert_rec = OrderInsertedRecord {
            seq: 1,
            ts_ns: 1000,
            symbol_id: 1,
            user_id: 100,
            order_id_hi: 0,
            order_id_lo: 1,
            price: Price(100),
            qty: Qty(10),
            side: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };

        state.ensure_book(
            insert_rec.symbol_id,
            insert_rec.price.0,
        );
        if let Some(book) =
            state.book_mut(insert_rec.symbol_id)
        {
            book.apply_insert_by_id(
                insert_rec.price.0,
                insert_rec.qty.0,
                insert_rec.side,
                insert_rec.user_id,
                insert_rec.ts_ns,
                insert_rec.order_id_hi,
                insert_rec.order_id_lo,
            );
        }

        if let Some(book) =
            state.book_mut(insert_rec.symbol_id)
        {
            let bbo = book.derive_bbo();
            assert!(bbo.is_some());
            let bbo = bbo.unwrap();
            assert_eq!(bbo.bid_px, 100);
            assert_eq!(bbo.bid_qty, 10);
        }
    });
}

#[test]
fn replay_events_fill_reduces_qty() {
    big_stack(|| {
        let mut state = new_state();
        let insert_rec = OrderInsertedRecord {
            seq: 1,
            ts_ns: 1000,
            symbol_id: 1,
            user_id: 100,
            order_id_hi: 0,
            order_id_lo: 1,
            price: Price(100),
            qty: Qty(10),
            side: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };

        state.ensure_book(
            insert_rec.symbol_id,
            insert_rec.price.0,
        );
        if let Some(book) =
            state.book_mut(insert_rec.symbol_id)
        {
            book.apply_insert_by_id(
                insert_rec.price.0,
                insert_rec.qty.0,
                insert_rec.side,
                insert_rec.user_id,
                insert_rec.ts_ns,
                insert_rec.order_id_hi,
                insert_rec.order_id_lo,
            );
        }

        let fill_rec = FillRecord {
            seq: 2,
            ts_ns: 2000,
            symbol_id: 1,
            taker_user_id: 200,
            maker_user_id: 100,
            _pad0: 0,
            taker_order_id_hi: 0,
            taker_order_id_lo: 2,
            maker_order_id_hi: 0,
            maker_order_id_lo: 1,
            price: Price(100),
            qty: Qty(5),
            taker_side: 1,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };

        if let Some(book) =
            state.book_mut(fill_rec.symbol_id)
        {
            book.apply_fill_by_order_id(
                fill_rec.maker_order_id_hi,
                fill_rec.maker_order_id_lo,
                fill_rec.qty.0,
                fill_rec.ts_ns,
            );
        }

        if let Some(book) =
            state.book_mut(fill_rec.symbol_id)
        {
            let bbo = book.derive_bbo();
            assert!(bbo.is_some());
            let bbo = bbo.unwrap();
            assert_eq!(bbo.bid_px, 100);
            assert_eq!(bbo.bid_qty, 5);
        }
    });
}

#[test]
fn replay_events_cancel_removes_order() {
    big_stack(|| {
        let mut state = new_state();
        let insert_rec = OrderInsertedRecord {
            seq: 1,
            ts_ns: 1000,
            symbol_id: 1,
            user_id: 100,
            order_id_hi: 0,
            order_id_lo: 1,
            price: Price(100),
            qty: Qty(10),
            side: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };

        state.ensure_book(
            insert_rec.symbol_id,
            insert_rec.price.0,
        );
        if let Some(book) =
            state.book_mut(insert_rec.symbol_id)
        {
            book.apply_insert_by_id(
                insert_rec.price.0,
                insert_rec.qty.0,
                insert_rec.side,
                insert_rec.user_id,
                insert_rec.ts_ns,
                insert_rec.order_id_hi,
                insert_rec.order_id_lo,
            );
        }

        let cancel_rec = OrderCancelledRecord {
            seq: 2,
            ts_ns: 2000,
            symbol_id: 1,
            user_id: 100,
            order_id_hi: 0,
            order_id_lo: 1,
            remaining_qty: Qty(10),
            reason: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };

        if let Some(book) =
            state.book_mut(cancel_rec.symbol_id)
        {
            book.apply_cancel_by_order_id(
                cancel_rec.order_id_hi,
                cancel_rec.order_id_lo,
                cancel_rec.ts_ns,
            );
        }

        if let Some(book) =
            state.book_mut(cancel_rec.symbol_id)
        {
            let bbo = book.derive_bbo();
            assert!(bbo.is_none());
        }
    });
}
