use rsx_types::NONE;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::TimeInForce;
use rsx_types::validate_order;

use crate::book::Orderbook;
use crate::event::Event;
use crate::event::FAIL_FOK;
use crate::event::FAIL_REDUCE_ONLY;
use crate::event::FAIL_VALIDATION;
use crate::event::REASON_CANCELLED;
use crate::event::REASON_FILLED;
use crate::user::update_positions_on_fill;

/// Incoming order descriptor (not yet in the book).
pub struct IncomingOrder {
    pub price: i64,
    pub qty: i64,
    pub remaining_qty: i64,
    pub side: Side,
    pub tif: TimeInForce,
    pub user_id: u32,
    pub reduce_only: bool,
    pub timestamp_ns: u64,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
}

pub fn process_new_order(
    book: &mut Orderbook,
    order: &mut IncomingOrder,
) {
    book.event_len = 0;
    let saved_event_len = book.event_len;

    // Validate
    if !validate_order(
        &book.config,
        Price(order.price),
        Qty(order.qty),
    ) {
        book.emit(Event::OrderFailed {
            user_id: order.user_id,
            reason: FAIL_VALIDATION,
        });
        return;
    }

    // Reduce-only enforcement
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
                    });
                    return;
                }
                let abs_pos =
                    nq.unsigned_abs() as i64;
                if order.remaining_qty > abs_pos {
                    order.remaining_qty = abs_pos;
                }
            }
        }
    }

    // Phase 1: Match against opposite side
    match order.side {
        Side::Buy => {
            while order.remaining_qty > 0
                && book.best_ask_tick != NONE
            {
                let ask_tick = book.best_ask_tick;
                let before = order.remaining_qty;
                match_at_level(
                    book, ask_tick, order,
                );
                let level = &book.active_levels
                    [ask_tick as usize];
                if level.order_count == 0 {
                    book.best_ask_tick =
                        book.scan_next_ask(ask_tick);
                }
                // No progress = no crossing orders
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
                match_at_level(
                    book, bid_tick, order,
                );
                let level = &book.active_levels
                    [bid_tick as usize];
                if level.order_count == 0 {
                    book.best_bid_tick =
                        book.scan_next_bid(bid_tick);
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

    // Phase 1.5: TIF enforcement
    if order.tif == TimeInForce::FOK
        && order.remaining_qty > 0
    {
        // Rollback events
        book.event_len = saved_event_len;
        book.emit(Event::OrderFailed {
            user_id: order.user_id,
            reason: FAIL_FOK,
        });
        return;
    }

    // Phase 2: Insert remainder or cancel
    if order.remaining_qty > 0 {
        if order.tif == TimeInForce::IOC {
            let filled = order.qty
                - order.remaining_qty;
            book.emit(Event::OrderDone {
                handle: NONE,
                user_id: order.user_id,
                reason: REASON_CANCELLED,
                filled_qty: Qty(filled),
                remaining_qty: Qty(
                    order.remaining_qty,
                ),
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
        // Fully filled
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
}

pub fn match_at_level(
    book: &mut Orderbook,
    tick: u32,
    aggressor: &mut IncomingOrder,
) {
    let mut cursor =
        book.active_levels[tick as usize].head;

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

        // Smooshed tick check: verify actual price
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

        let fill_qty = aggressor
            .remaining_qty
            .min(maker_qty);

        aggressor.remaining_qty -= fill_qty;
        let maker_slot =
            book.orders.get_mut(cursor);
        maker_slot.remaining_qty.0 -= fill_qty;
        let maker_remaining =
            maker_slot.remaining_qty.0;

        book.active_levels[tick as usize]
            .total_qty -= fill_qty;

        book.emit(Event::Fill {
            maker_handle: cursor,
            taker_user_id: aggressor.user_id,
            price: Price(maker_price),
            qty: Qty(fill_qty),
            side: aggressor.side as u8,
            maker_order_id_hi: maker_oid_hi,
            maker_order_id_lo: maker_oid_lo,
            taker_order_id_hi: aggressor.order_id_hi,
            taker_order_id_lo: aggressor.order_id_lo,
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
            // Fully filled: unlink and free
            let prev =
                book.orders.get(cursor).prev;
            let next =
                book.orders.get(cursor).next;
            let level = &mut book.active_levels
                [tick as usize];

            if prev != NONE {
                book.orders.get_mut(prev).next =
                    next;
            } else {
                level.head = next;
            }
            if next != NONE {
                book.orders.get_mut(next).prev =
                    prev;
            } else {
                level.tail = prev;
            }
            level.order_count -= 1;

            // Emit OrderDone for maker
            let orig_qty = book.orders
                .get(cursor)
                .original_qty;
            book.emit(Event::OrderDone {
                handle: cursor,
                user_id: maker_user_id,
                reason: REASON_FILLED,
                filled_qty: orig_qty,
                remaining_qty: Qty(0),
                order_id_hi: maker_oid_hi,
                order_id_lo: maker_oid_lo,
            });

            book.orders
                .get_mut(cursor)
                .set_active(false);
            book.orders.free(cursor);

            // Decrement maker order count
            if let Some(&uidx) =
                book.user_map.get(&maker_user_id)
            {
                book.user_states[uidx as usize]
                    .order_count -= 1;
            }
        }

        cursor = next_cursor;
    }
}
