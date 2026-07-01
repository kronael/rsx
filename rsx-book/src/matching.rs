use rsx_types::NONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::TimeInForce;
use rsx_types::validate_order;
use crate::book::Orderbook;
use crate::event::CANCEL_POST_ONLY;
use crate::event::Event;
use crate::event::FAIL_FOK;
use crate::event::FAIL_REDUCE_ONLY;
use crate::event::FAIL_VALIDATION;
use crate::event::REASON_CANCELLED;
use crate::event::REASON_FILLED;
use crate::user::update_positions_on_fill;

pub struct IncomingOrder {
    pub price: i64,
    pub qty: i64,
    pub remaining_qty: i64,
    pub side: Side,
    pub tif: TimeInForce,
    pub user_id: u32,
    pub reduce_only: bool,
    pub post_only: bool,
    pub timestamp_ns: u64,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
}

/// Invariant #2 (exactly-one completion): each path through this
/// function emits exactly one terminal event for the incoming order —
/// `OrderFailed` (validation / FOK / reduce-only), `OrderCancelled`
/// (post-only would cross), `OrderDone` (IOC residual or full fill),
/// or `OrderInserted` (resting; terminal event fires later on
/// fill/cancel). Invariant #6 (no crossed book): we match against
/// `best_ask_tick` / `best_bid_tick` until the aggressor stops crossing,
/// then insert any residual via `insert_resting`, so post-loop the
/// book cannot be crossed.
pub fn process_new_order(
    book: &mut Orderbook,
    order: &mut IncomingOrder,
) {
    book.event_len = 0;
    let old_bid = book.best_bid_tick;
    let old_ask = book.best_ask_tick;

    if !validate_order(
        &book.config,
        Price(order.price),
        Qty(order.qty),
    ) {
        book.emit(Event::OrderFailed {
            user_id: order.user_id,
            reason: FAIL_VALIDATION,
            order_id_hi: order.order_id_hi,
            order_id_lo: order.order_id_lo,
        });
        return;
    }

    if order.reduce_only {
        let net = book
            .user_map
            .get(&order.user_id)
            .map(|&idx| {
                book.user_states[idx as usize].net_qty
            });
        match net {
            None => {
                book.emit(Event::OrderFailed {
                    user_id: order.user_id,
                    reason: FAIL_REDUCE_ONLY,
                    order_id_hi: order.order_id_hi,
                    order_id_lo: order.order_id_lo,
                });
                return;
            }
            Some(nq) => {
                let reject = match order.side {
                    Side::Buy => nq >= 0,
                    Side::Sell => nq <= 0,
                };
                if reject {
                    book.emit(Event::OrderFailed {
                        user_id: order.user_id,
                        reason: FAIL_REDUCE_ONLY,
                        order_id_hi: order.order_id_hi,
                        order_id_lo: order.order_id_lo,
                    });
                    return;
                }
                let abs_pos = nq.unsigned_abs()
                    .min(i64::MAX as u64) as i64;
                if order.remaining_qty > abs_pos {
                    order.remaining_qty = abs_pos;
                }
            }
        }
    }

    if order.post_only {
        // Cross detection by RAW PRICE: the compression index is a
        // sawtooth and cannot be compared as a price proxy.
        let would_cross = match order.side {
            Side::Buy => {
                book.best_ask_tick != NONE
                    && order.price >= book.best_ask_px
            }
            Side::Sell => {
                book.best_bid_tick != NONE
                    && order.price <= book.best_bid_px
            }
        };
        if would_cross {
            book.emit(Event::OrderCancelled {
                handle: NONE,
                user_id: order.user_id,
                remaining_qty: Qty(order.remaining_qty),
                reason: CANCEL_POST_ONLY,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            });
            return;
        }
    }

    if order.tif == TimeInForce::FOK {
        let avail = available_liquidity(
            book, order.side, order.price,
        );
        if avail < order.remaining_qty {
            book.emit(Event::OrderFailed {
                user_id: order.user_id,
                reason: FAIL_FOK,
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            });
            return;
        }
    }

    match order.side {
        Side::Buy => {
            while order.remaining_qty > 0
                && book.best_ask_tick != NONE
            {
                let ask_tick = book.best_ask_tick;
                let before = order.remaining_qty;
                match_at_level(book, ask_tick, order);
                let level = &book.active_levels
                    [ask_tick as usize];
                if level.order_count == 0 {
                    book.best_ask_tick =
                        book.scan_next_ask(ask_tick);
                    book.best_ask_px = book
                        .price_at_tick(book.best_ask_tick);
                }
                if order.remaining_qty == before {
                    break;
                }
                if book.best_ask_tick == NONE {
                    break;
                }
            }
        }
        Side::Sell => {
            while order.remaining_qty > 0
                && book.best_bid_tick != NONE
            {
                let bid_tick = book.best_bid_tick;
                let before = order.remaining_qty;
                match_at_level(book, bid_tick, order);
                let level = &book.active_levels
                    [bid_tick as usize];
                if level.order_count == 0 {
                    book.best_bid_tick =
                        book.scan_next_bid(bid_tick);
                    book.best_bid_px = book
                        .price_at_tick(book.best_bid_tick);
                }
                if order.remaining_qty == before {
                    break;
                }
                if book.best_bid_tick == NONE {
                    break;
                }
            }
        }
    }

    if order.remaining_qty > 0 {
        if order.tif == TimeInForce::IOC {
            let filled = order.qty - order.remaining_qty;
            book.emit(Event::OrderDone {
                handle: NONE,
                user_id: order.user_id,
                reason: REASON_CANCELLED,
                filled_qty: Qty(filled),
                remaining_qty: Qty(order.remaining_qty),
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            });
        } else {
            let handle = book.insert_resting(
                order.price,
                order.remaining_qty,
                order.side,
                order.tif as u8,
                order.user_id,
                order.reduce_only,
                order.timestamp_ns,
                order.order_id_hi,
                order.order_id_lo,
            );
            book.emit(Event::OrderInserted {
                handle,
                user_id: order.user_id,
                side: order.side as u8,
                price: Price(order.price),
                qty: Qty(order.remaining_qty),
                order_id_hi: order.order_id_hi,
                order_id_lo: order.order_id_lo,
            });
        }
    } else {
        book.emit(Event::OrderDone {
            handle: NONE,
            user_id: order.user_id,
            reason: REASON_FILLED,
            filled_qty: Qty(order.qty),
            remaining_qty: Qty(0),
            order_id_hi: order.order_id_hi,
            order_id_lo: order.order_id_lo,
        });
    }

    // Emit BBO if best bid or ask changed
    if book.best_bid_tick != old_bid
        || book.best_ask_tick != old_ask
    {
        emit_bbo(book);
    }
}

fn emit_bbo(book: &mut Orderbook) {
    let (bid_px, bid_qty) =
        if book.best_bid_tick != NONE {
            let lvl = &book.active_levels
                [book.best_bid_tick as usize];
            let px =
                book.orders.get(lvl.head).price.0;
            (px, lvl.total_qty)
        } else {
            (0, 0)
        };
    let (ask_px, ask_qty) =
        if book.best_ask_tick != NONE {
            let lvl = &book.active_levels
                [book.best_ask_tick as usize];
            let px =
                book.orders.get(lvl.head).price.0;
            (px, lvl.total_qty)
        } else {
            (0, 0)
        };
    book.emit(Event::BBO {
        bid_px: Price(bid_px),
        bid_qty: Qty(bid_qty),
        ask_px: Price(ask_px),
        ask_qty: Qty(ask_qty),
    });
}

/// Match an aggressor against resting orders at a single price level.
///
/// Invariant #1 (Fills precede ORDER_DONE): `Event::Fill` is emitted
/// inside the inner loop *before* the `Event::OrderDone` for any maker
/// that fully fills, and before `process_new_order` emits the taker's
/// terminal event. Invariant #3 (FIFO): walks the level from
/// `level.head` set by `insert_resting`.
pub fn match_at_level(
    book: &mut Orderbook,
    tick: u32,
    aggressor: &mut IncomingOrder,
) {
    let mut cursor = book.active_levels[tick as usize].head;

    while cursor != NONE
        && aggressor.remaining_qty > 0
    {
        let maker = book.orders.get(cursor);
        let maker_price = maker.price.0;
        let maker_qty = maker.remaining_qty.0;
        let maker_user_id = maker.user_id;
        let maker_oid_hi = maker.order_id_hi;
        let maker_oid_lo = maker.order_id_lo;
        let next_cursor = maker.next;

        match aggressor.side {
            Side::Buy => {
                if maker_price > aggressor.price {
                    cursor = next_cursor;
                    continue;
                }
            }
            Side::Sell => {
                if maker_price < aggressor.price {
                    cursor = next_cursor;
                    continue;
                }
            }
        }

        let fill_qty =
            aggressor.remaining_qty.min(maker_qty);

        debug_assert!(
            maker_price
                .checked_mul(fill_qty)
                .is_some(),
            "fill notional overflow"
        );

        aggressor.remaining_qty -= fill_qty;
        let maker_slot = book.orders.get_mut(cursor);
        maker_slot.remaining_qty.0 -= fill_qty;
        let maker_remaining = maker_slot.remaining_qty.0;

        book.active_levels[tick as usize]
            .total_qty = book.active_levels
            [tick as usize]
            .total_qty
            .saturating_sub(fill_qty);

        book.emit(Event::Fill {
            maker_handle: cursor,
            maker_user_id,
            taker_user_id: aggressor.user_id,
            price: Price(maker_price),
            qty: Qty(fill_qty),
            side: aggressor.side as u8,
            maker_order_id_hi: maker_oid_hi,
            maker_order_id_lo: maker_oid_lo,
            taker_order_id_hi: aggressor.order_id_hi,
            taker_order_id_lo: aggressor.order_id_lo,
            taker_ts_ns: aggressor.timestamp_ns,
        });

        update_positions_on_fill(
            &mut book.user_states,
            &mut book.user_map,
            &mut book.user_free_list,
            &mut book.user_bump,
            aggressor.user_id,
            maker_user_id,
            aggressor.side,
            fill_qty,
        );

        if maker_remaining == 0 {
            book.unlink_order(cursor);

            let orig_qty =
                book.orders.get(cursor).original_qty;
            book.emit(Event::OrderDone {
                handle: cursor,
                user_id: maker_user_id,
                reason: REASON_FILLED,
                filled_qty: orig_qty,
                remaining_qty: Qty(0),
                order_id_hi: maker_oid_hi,
                order_id_lo: maker_oid_lo,
            });

            book.orders.get_mut(cursor).set_active(false);
            book.orders.free(cursor);

            if let Some(&uidx) =
                book.user_map.get(&maker_user_id)
            {
                book.user_states[uidx as usize]
                    .order_count =
                    book.user_states[uidx as usize]
                        .order_count
                        .saturating_sub(1);
            }
        }

        cursor = next_cursor;
    }
}

/// Total resting qty on the opposite side that an aggressor at
/// `limit_price` would cross. The compression map is a sawtooth, so we
/// cannot walk levels in price order via `scan_next_*`; instead do a
/// single bounded pass over all levels and sum orders that satisfy the
/// price predicate. No allocation.
fn available_liquidity(
    book: &Orderbook,
    side: Side,
    limit_price: i64,
) -> i64 {
    let mut total: i64 = 0;
    for level in book.active_levels.iter() {
        if level.order_count == 0 {
            continue;
        }
        let mut cursor = level.head;
        while cursor != NONE {
            let maker = book.orders.get(cursor);
            let crosses = match side {
                // aggressor buys asks priced <= limit
                Side::Buy => {
                    maker.side == Side::Sell as u8
                        && maker.price.0 <= limit_price
                }
                // aggressor sells into bids priced >= limit
                Side::Sell => {
                    maker.side == Side::Buy as u8
                        && maker.price.0 >= limit_price
                }
            };
            if crosses {
                total += maker.remaining_qty.0;
            }
            cursor = maker.next;
        }
    }
    total
}
