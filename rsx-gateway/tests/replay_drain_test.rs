//! `rsx_gateway::drain_replay` smoke test. Stands up a real
//! `ReplicationService` against a WAL, drains it, verifies
//! apply was invoked and the returned `new_tip` matches.

mod common;

use common::replay::serve_wal;
use rsx_gateway::drain_replay;
use rsx_messages::FillRecord;
use rsx_messages::GW_STREAM_ID;
use rsx_types::Price;
use rsx_types::Qty;

const STREAM_ID: u32 = GW_STREAM_ID;

fn fill(seq: u64) -> FillRecord {
    FillRecord {
        seq,
        ts_ns: seq,
        symbol_id: 1,
        taker_user_id: 1,
        maker_user_id: 2,
        _pad0: 0,
        taker_order_id_hi: 0,
        taker_order_id_lo: seq,
        maker_order_id_hi: 0,
        maker_order_id_lo: 100 + seq,
        price: Price(100),
        qty: Qty(1),
        taker_side: 0,
        reduce_only: 0,
        tif: 0,
        post_only: 0,
        _pad1: [0; 4],
        gw_in_ns: 0,
        risk_in_ns: 0,
        me_in_ns: 0,
        match_done_ns: 0,
        gw_out_ns: 0,
    }
}

#[test]
fn drain_replay_walks_wal_and_invokes_apply() {
    let server = serve_wal((1..=3).map(fill).collect());
    assert_eq!(server.last_seq, 3);

    let mut applied_seqs: Vec<u64> = Vec::new();
    let new_tip = drain_replay(
        STREAM_ID,
        server.addr.to_string(),
        0,
        server.tip_file.clone(),
        server.tls.clone(),
        |raw| {
            let seq = rsx_cast::wal::extract_seq(&raw.payload).unwrap_or(0);
            applied_seqs.push(seq);
        },
    )
    .expect("drain failed");

    assert_eq!(new_tip, server.last_seq);
    assert_eq!(applied_seqs, vec![1, 2, 3]);
}
