use crate::state::MarketDataState;
use crate::wire::encode_bbo;
use crate::wire::encode_l2_delta;
use crate::wire::encode_trade;
use rsx_cast::decode_payload;
use rsx_messages::FillRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_INSERTED;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Folds one ME record into the shadow book and pushes the resulting
/// L2, BBO and trade frames to subscribers. The live stream, the boot
/// bootstrap and the gap replay all go through here; other record
/// types and undecodable payloads are ignored.
pub fn apply_record(
    state: &Rc<RefCell<MarketDataState>>,
    record_type: u16,
    payload: &[u8],
    max_outbound: usize,
) {
    match record_type {
        RECORD_ORDER_INSERTED => {
            if let Some(rec) = decode_payload::<OrderInsertedRecord>(payload) {
                apply_insert(state, &rec, max_outbound);
            }
        }
        RECORD_ORDER_CANCELLED => {
            if let Some(rec) = decode_payload::<OrderCancelledRecord>(payload) {
                apply_cancel(state, &rec, max_outbound);
            }
        }
        RECORD_FILL => {
            if let Some(rec) = decode_payload::<FillRecord>(payload) {
                apply_fill(state, &rec, max_outbound);
            }
        }
        _ => {}
    }
}

fn apply_insert(
    state: &Rc<RefCell<MarketDataState>>,
    rec: &OrderInsertedRecord,
    max_outbound: usize,
) {
    let mut st = state.borrow_mut();
    st.ensure_book(rec.symbol_id, rec.price.0);
    let (side, price) = match st.book_mut(rec.symbol_id) {
        Some(book) => {
            book.apply_insert_by_id(
                rec.price.0,
                rec.qty.0,
                rec.side,
                rec.user_id,
                rec.ts_ns,
                rec.order_id_hi,
                rec.order_id_lo,
            );
            book.maybe_recenter();
            (rec.side, rec.price.0)
        }
        None => return,
    };
    broadcast_updates(&mut st, rec.symbol_id, side, price, max_outbound);
}

fn apply_cancel(
    state: &Rc<RefCell<MarketDataState>>,
    rec: &OrderCancelledRecord,
    max_outbound: usize,
) {
    let mut st = state.borrow_mut();
    let update = match st.book_mut(rec.symbol_id) {
        Some(book) => book.apply_cancel_by_order_id(rec.order_id_hi, rec.order_id_lo, rec.ts_ns),
        None => None,
    };
    if let Some((side, price)) = update {
        broadcast_updates(&mut st, rec.symbol_id, side, price, max_outbound);
    }
}

fn apply_fill(state: &Rc<RefCell<MarketDataState>>, rec: &FillRecord, max_outbound: usize) {
    let mut st = state.borrow_mut();
    let mut trade_msg: Option<Arc<[u8]>> = None;
    let update = match st.book_mut(rec.symbol_id) {
        Some(book) => {
            let update = book.apply_fill_by_order_id(
                rec.maker_order_id_hi,
                rec.maker_order_id_lo,
                rec.qty.0,
                rec.ts_ns,
            );
            if update.is_some() {
                let trade = book.make_trade(rec.price.0, rec.qty.0, rec.taker_side, rec.ts_ns);
                trade_msg = Some(encode_trade(&trade).into());
            }
            update
        }
        None => None,
    };
    if let Some((side, price)) = update {
        broadcast_updates(&mut st, rec.symbol_id, side, price, max_outbound);
    }
    if let Some(msg) = trade_msg {
        let clients = st.clients_for_symbol(rec.symbol_id);
        for client_id in clients {
            if st.has_trades(client_id, rec.symbol_id)
                // HEAP: fan-out clone of the protobuf trade frame per
                // trades subscriber (push_to_client takes an owned
                // Arc<[u8]>). One Arc bump per subscriber per trade.
                && !st.push_to_client(
                    client_id,
                    msg.clone(),
                    max_outbound,
                )
            {
                let depth = st.client_depth(client_id);
                st.send_snapshot_to_client(client_id, rec.symbol_id, depth, max_outbound);
            }
        }
    }
}

fn broadcast_updates(
    st: &mut MarketDataState,
    symbol_id: u32,
    side: u8,
    price: i64,
    max_outbound: usize,
) {
    let (delta, bbo) = match st.book_mut(symbol_id) {
        Some(book) => {
            let delta = book.derive_l2_delta(side, price);
            let bbo = book.derive_bbo();
            (delta, bbo)
        }
        None => return,
    };
    let delta_msg: Arc<[u8]> = encode_l2_delta(&delta).into();
    let mut bbo_msg: Option<Arc<[u8]>> = None;
    if let Some(bbo) = bbo {
        let changed = match st.last_bbo_mut(symbol_id) {
            Some(last) => {
                if last.as_ref() != Some(&bbo) {
                    *last = Some(bbo.clone());
                    true
                } else {
                    false
                }
            }
            None => true,
        };
        if changed {
            bbo_msg = Some(encode_bbo(&bbo).into());
        }
    }

    let clients = st.clients_for_symbol(symbol_id);
    for client_id in clients {
        if st.has_depth(client_id, symbol_id)
            // HEAP: fan-out clone of the protobuf L2 delta frame per
            // depth subscriber (push_to_client takes an owned Arc<[u8]>).
            && !st.push_to_client(
                client_id,
                delta_msg.clone(),
                max_outbound,
            )
        {
            st.send_snapshot_to_client(
                client_id,
                symbol_id,
                st.client_depth(client_id),
                max_outbound,
            );
        }
        if let Some(ref msg) = bbo_msg {
            if st.has_bbo(client_id, symbol_id) {
                // HEAP: fan-out clone of the protobuf BBO frame per BBO
                // subscriber; a full outbound drops it, the next BBO
                // supersedes.
                let _accepted = st.push_to_client(client_id, msg.clone(), max_outbound);
            }
        }
    }
}
