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

    if order.tif == TimeInForce::FOK
        && !can_fill_fully(
            book,
            order.side,
            order.price,
            order.remaining_qty,
        )
    {
        book.emit(Event::OrderFailed {
            user_id: order.user_id,
            reason: FAIL_FOK,
            order_id_hi: order.order_id_hi,
            order_id_lo: order.order_id_lo,
        });
        return;
    }

    // Effective size the order will actually try to fill: the original
    // qty, except a reduce-only order clamped to the position above. Fills
    // are measured against THIS, not `order.qty`, so the reduce-only clamp
    // is never miscounted as execution (see the residual/`filled` sites).
    let fillable = order.remaining_qty;

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
        match order.tif {
            TimeInForce::IOC => {
                let filled = fillable - order.remaining_qty;
                book.emit(Event::OrderDone {
                    handle: NONE,
                    user_id: order.user_id,
                    reason: REASON_CANCELLED,
                    filled_qty: Qty(filled),
                    remaining_qty: Qty(order.remaining_qty),
                    order_id_hi: order.order_id_hi,
                    order_id_lo: order.order_id_lo,
                });
            }
            TimeInForce::FOK => {
                // The FOK pre-check (`can_fill_fully`) is exact, so a FOK
                // that entered the match loop always fully fills — a
                // residual here means feasibility and matching disagreed.
                // Defense in depth: a FOK is all-or-nothing and must NEVER
                // rest, so reject rather than fall through to
                // `insert_resting`. `can_fill_fully` guarantees no fills
                // were emitted on this path.
                debug_assert!(
                    false,
                    "FOK residual after can_fill_fully passed: \
                     remaining={} of {}",
                    order.remaining_qty, fillable,
                );
                book.emit(Event::OrderFailed {
                    user_id: order.user_id,
                    reason: FAIL_FOK,
                    order_id_hi: order.order_id_hi,
                    order_id_lo: order.order_id_lo,
                });
            }
            TimeInForce::GTC => {
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
        }
    } else {
        book.emit(Event::OrderDone {
            handle: NONE,
            user_id: order.user_id,
            reason: REASON_FILLED,
            filled_qty: Qty(fillable),
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

/// True iff a FOK aggressor at `limit_price` can be filled completely —
/// i.e. the resting qty on the opposite side that crosses `limit_price`
/// is at least `needed`. This is the "try to match it" question: walk the
/// crossing levels in price order (the same order a match consumes them)
/// and stop the instant the running total reaches `needed`.
///
/// Zone 0 holds one raw price per slot, so a whole level either crosses
/// or does not — the maintained `total_qty` counts it in O(1), the happy
/// near-BBO path. Compressed zones (≥1, and the zone-4 catch-all) pack
/// DISTINCT raw prices into one level, so the `total_qty` shortcut would
/// over-count non-crossing makers → feasibility would wrongly pass and a
/// FOK could rest. There we walk the level's orders and sum only those
/// whose actual raw price crosses `limit_price`. Early-exit is preserved:
/// bands are visited in price order, so once a whole level sits beyond the
/// limit (its nearest-to-mid order does not cross) all further levels are
/// beyond it too — stop. O(levels + orders actually crossed).
fn can_fill_fully(
    book: &Orderbook,
    side: Side,
    limit_price: i64,
    needed: i64,
) -> bool {
    let mut total: i64 = 0;
    let zone0_end = book.compression.zone_slots[0];
    match side {
        // Aggressor buys: cross asks priced <= limit, ascending price.
        Side::Buy => {
            for &(lo, hi) in book.price_asc.iter() {
                let mut cur = lo;
                while let Some(t) =
                    book.ask_occ.find_first_in(cur, hi)
                {
                    let lvl =
                        &book.active_levels[t as usize];
                    if t < zone0_end {
                        // Single price per slot: total_qty is exact.
                        if book.orders.get(lvl.head).price.0
                            > limit_price
                        {
                            return false; // asks now above limit
                        }
                        total += lvl.total_qty;
                        if total >= needed {
                            return true;
                        }
                    } else {
                        // Compressed: sum only orders that truly cross.
                        let mut min_px = i64::MAX;
                        let mut c = lvl.head;
                        while c != NONE {
                            let o = book.orders.get(c);
                            let px = o.price.0;
                            if px < min_px {
                                min_px = px;
                            }
                            if px <= limit_price {
                                total += o.remaining_qty.0;
                                if total >= needed {
                                    return true;
                                }
                            }
                            c = o.next;
                        }
                        if min_px > limit_price {
                            return false; // whole band above limit
                        }
                    }
                    cur = t + 1;
                }
            }
        }
        // Aggressor sells: cross bids priced >= limit, descending price.
        Side::Sell => {
            for &(lo, hi) in book.price_asc.iter().rev() {
                let mut top = hi;
                while let Some(t) =
                    book.bid_occ.find_last_in(lo, top)
                {
                    let lvl =
                        &book.active_levels[t as usize];
                    if t < zone0_end {
                        // Single price per slot: total_qty is exact.
                        if book.orders.get(lvl.head).price.0
                            < limit_price
                        {
                            return false; // bids now below limit
                        }
                        total += lvl.total_qty;
                        if total >= needed {
                            return true;
                        }
                    } else {
                        // Compressed: sum only orders that truly cross.
                        let mut max_px = i64::MIN;
                        let mut c = lvl.head;
                        while c != NONE {
                            let o = book.orders.get(c);
                            let px = o.price.0;
                            if px > max_px {
                                max_px = px;
                            }
                            if px >= limit_price {
                                total += o.remaining_qty.0;
                                if total >= needed {
                                    return true;
                                }
                            }
                            c = o.next;
                        }
                        if max_px < limit_price {
                            return false; // whole band below limit
                        }
                    }
                    top = t;
                }
            }
        }
    }
    false
}
