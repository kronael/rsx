use crate::book::Orderbook;
use crate::event::Event;
use crate::event::CANCEL_POST_ONLY;
use crate::event::FAIL_FOK;
use crate::event::FAIL_REDUCE_ONLY;
use crate::event::FAIL_VALIDATION;
use crate::event::REASON_CANCELLED;
use crate::event::REASON_FILLED;
use rsx_types::validate_order;
use rsx_types::Price;
use rsx_types::Qty;
use rsx_types::Side;
use rsx_types::TimeInForce;
use rsx_types::NONE;

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
pub fn process_new_order(book: &mut Orderbook, order: &mut IncomingOrder) {
    book.event_len = 0;
    let old_bid = book.best_bid_tick;
    let old_ask = book.best_ask_tick;
    // Track px too: in a compressed slot the best price can move while the
    // tick stays the same, and that still warrants a fresh BBO.
    let old_bid_px = book.best_bid_px;
    let old_ask_px = book.best_ask_px;

    if !validate_order(&book.config, Price(order.price), Qty(order.qty)) {
        book.emit(Event::OrderFailed {
            user_id: order.user_id,
            reason: FAIL_VALIDATION,
            order_id_hi: order.order_id_hi,
            order_id_lo: order.order_id_lo,
        });
        return;
    }

    if order.reduce_only {
        let net = book.users.net_qty(order.user_id);
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
                let abs_pos = nq.unsigned_abs().min(i64::MAX as u64) as i64;
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
            Side::Buy => book.best_ask_tick != NONE && order.price >= book.best_ask_px,
            Side::Sell => book.best_bid_tick != NONE && order.price <= book.best_bid_px,
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
        && !can_fill_fully(book, order.side, order.price, order.remaining_qty)
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
            while order.remaining_qty > 0 && book.best_ask_tick != NONE {
                let before = order.remaining_qty;
                match_at_level(book, book.best_ask_tick, order);
                // The matched slot may have lost only its best-priced
                // sells (compressed/mixed slot) while others survive, so
                // recompute best ask from per-side occupancy rather than
                // testing for a fully-empty level (BOOK-STALE-BBA /
                // BOOK-STALE-OCC-ME-CRASH).
                book.refresh_best_ask();
                if order.remaining_qty == before {
                    break; // best ask no longer crosses the taker limit
                }
            }
        }
        Side::Sell => {
            while order.remaining_qty > 0 && book.best_bid_tick != NONE {
                let before = order.remaining_qty;
                match_at_level(book, book.best_bid_tick, order);
                book.refresh_best_bid();
                if order.remaining_qty == before {
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

    // Emit BBO if best bid or ask changed (tick OR price).
    if book.best_bid_tick != old_bid
        || book.best_ask_tick != old_ask
        || book.best_bid_px != old_bid_px
        || book.best_ask_px != old_ask_px
    {
        emit_bbo(book);
    }
}

fn emit_bbo(book: &mut Orderbook) {
    // Tripwire: a best tick must point at a level actually resting the
    // tracked side (per-side occupancy must be exact). A stale bit here
    // is what drove BOOK-STALE-OCC-ME-CRASH (head == NONE deref).
    debug_assert!(
        book.best_bid_tick == NONE || book.active_levels[book.best_bid_tick as usize].bid_count > 0,
        "best_bid_tick points at a level with no resting buy",
    );
    debug_assert!(
        book.best_ask_tick == NONE || book.active_levels[book.best_ask_tick as usize].ask_count > 0,
        "best_ask_tick points at a level with no resting sell",
    );
    let (bid_px, bid_qty, ask_px, ask_qty) = book.current_bbo();
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
pub fn match_at_level(book: &mut Orderbook, tick: u32, aggressor: &mut IncomingOrder) {
    let mut cursor = book.active_levels[tick as usize].head;

    while cursor != NONE && aggressor.remaining_qty > 0 {
        let maker = book.orders.get(cursor);
        let maker_price = maker.price.0;
        let maker_side = maker.side;
        let maker_qty = maker.remaining_qty.0;
        let maker_user_id = maker.user_id;
        let maker_oid_hi = maker.order_id_hi;
        let maker_oid_lo = maker.order_id_lo;
        let next_cursor = maker.next;

        // A compressed slot can hold BOTH sides and multiple raw prices
        // (BOOK-MIXED-SIDE-SELF-TRADE), so match ONLY an opposite-side
        // maker whose actual price crosses the taker limit. Same-side or
        // non-crossing makers are left resting — one comparison per walked
        // order, no extra work on the zone-0 happy path.
        let crosses = maker_side != aggressor.side as u8
            && match aggressor.side {
                Side::Buy => maker_price <= aggressor.price,
                Side::Sell => maker_price >= aggressor.price,
            };
        if !crosses {
            cursor = next_cursor;
            continue;
        }

        let fill_qty = aggressor.remaining_qty.min(maker_qty);

        debug_assert!(
            maker_price.checked_mul(fill_qty).is_some(),
            "fill notional overflow"
        );

        aggressor.remaining_qty -= fill_qty;
        let maker_slot = book.orders.get_mut(cursor);
        maker_slot.remaining_qty.0 -= fill_qty;
        let maker_remaining = maker_slot.remaining_qty.0;

        book.active_levels[tick as usize].total_qty = book.active_levels[tick as usize]
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
            gw_in_ns: aggressor.timestamp_ns,
        });

        book.users
            .apply_fill(aggressor.user_id, maker_user_id, aggressor.side, fill_qty);

        if maker_remaining == 0 {
            book.unlink_order(cursor);

            let orig_qty = book.orders.get(cursor).original_qty;
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

            book.users.remove_order(maker_user_id);
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
fn can_fill_fully(book: &Orderbook, side: Side, limit_price: i64, needed: i64) -> bool {
    let mut total: i64 = 0;
    let zone0_end = book.compression.zone_slots[0];
    match side {
        // Aggressor buys: cross asks priced <= limit, ascending price.
        Side::Buy => {
            for &(lo, hi) in book.price_asc.iter() {
                let mut cur = lo;
                while let Some(t) = book.ask_occ.find_first_in(cur, hi) {
                    let lvl = &book.active_levels[t as usize];
                    if t < zone0_end {
                        // Single price per slot: total_qty is exact.
                        if book.orders.get(lvl.head).price.0 > limit_price {
                            return false; // asks now above limit
                        }
                        total += lvl.total_qty;
                        if total >= needed {
                            return true;
                        }
                    } else {
                        // Compressed slot: mixed sides + prices. Count only
                        // opposite-side (sell) qty whose actual price
                        // crosses — the same predicate as `match_at_level`,
                        // so feasibility == the fill (a resting BUY in the
                        // slot must NOT count as available for a buy taker).
                        let mut min_sell = i64::MAX;
                        let mut c = lvl.head;
                        while c != NONE {
                            let o = book.orders.get(c);
                            if o.side == Side::Sell as u8 {
                                let px = o.price.0;
                                if px < min_sell {
                                    min_sell = px;
                                }
                                if px <= limit_price {
                                    total += o.remaining_qty.0;
                                    if total >= needed {
                                        return true;
                                    }
                                }
                            }
                            c = o.next;
                        }
                        if min_sell > limit_price {
                            return false; // every sell in this band is above
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
                while let Some(t) = book.bid_occ.find_last_in(lo, top) {
                    let lvl = &book.active_levels[t as usize];
                    if t < zone0_end {
                        // Single price per slot: total_qty is exact.
                        if book.orders.get(lvl.head).price.0 < limit_price {
                            return false; // bids now below limit
                        }
                        total += lvl.total_qty;
                        if total >= needed {
                            return true;
                        }
                    } else {
                        // Compressed slot: mixed sides + prices. Count only
                        // opposite-side (buy) qty whose actual price crosses
                        // — same predicate as `match_at_level`.
                        let mut max_buy = i64::MIN;
                        let mut c = lvl.head;
                        while c != NONE {
                            let o = book.orders.get(c);
                            if o.side == Side::Buy as u8 {
                                let px = o.price.0;
                                if px > max_buy {
                                    max_buy = px;
                                }
                                if px >= limit_price {
                                    total += o.remaining_qty.0;
                                    if total >= needed {
                                        return true;
                                    }
                                }
                            }
                            c = o.next;
                        }
                        if max_buy < limit_price {
                            return false; // every buy in this band is below
                        }
                    }
                    top = t;
                }
            }
        }
    }
    false
}
