use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::Framed;
use rsx_messages::BboRecord;
use rsx_messages::MarkPriceRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_MARK_PRICE;
use rsx_risk::config::LiquidationConfig;
use rsx_risk::config::ReplicationConfig;
use rsx_risk::config::ShardConfig;
use rsx_risk::funding::FundingConfig;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::rings::OrderResponse;
use rsx_risk::rings::ShardRings;
use rsx_risk::shard::RiskShard;
use rsx_risk::types::BboUpdate;
use rtrb::RingBuffer;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn test_config(max_symbols: usize) -> ShardConfig {
    ShardConfig {
        shard_id: 0,
        shard_count: 1,
        max_symbols,
        symbol_params: vec![
            SymbolRiskParams {
                initial_margin_rate: 1000,
                maintenance_margin_rate: 500,
                max_leverage: 10,
            };
            max_symbols
        ],
        taker_fee_bps: vec![5; max_symbols],
        maker_fee_bps: vec![-1; max_symbols],
        funding_config: FundingConfig::default(),
        liquidation_config: LiquidationConfig::default(),
        replication_config: ReplicationConfig {
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
        },
    }
}

fn make_rings() -> ShardRings {
    let (resp_prod, _resp_cons) = RingBuffer::<OrderResponse>::new(8);
    let (acc_prod, _acc_cons) = RingBuffer::<rsx_risk::types::OrderRequest>::new(8);

    ShardRings {
        response_producer: resp_prod,
        accepted_producer: acc_prod,
    }
}

#[test]
fn mark_cast_updates_risk_mark_prices() {
    let mut shard = RiskShard::new(test_config(2));
    let mut rings = make_rings();

    let _recv_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let tmp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_local = tmp.local_addr().unwrap();
    drop(tmp);

    let wal_dir = PathBuf::from("./tmp/cmp_mark_test");
    let mut sender = CastSender::new(recv_local, 0, &wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver = CastReceiver::new(recv_local, sender_addr).unwrap();

    let mut rec = MarkPriceRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: 1,
        _pad0: 0,
        mark_price: rsx_types::Price(55_000),
        source_mask: 0,
        source_count: 1,
        _pad1: [0; 24],
    };
    sender.send_framed(&Framed::pack(&mut rec, 1)).unwrap();

    thread::sleep(Duration::from_millis(10));
    loop {
        let (hdr, payload) = match receiver.try_recv() {
            CastRecv::Data(h, p) => (h, p),
            CastRecv::Empty => break,
            CastRecv::Faulted { .. } => panic!("unexpected fault"),
            CastRecv::Reconnect { .. } => panic!("unexpected reconnect"),
        };
        if hdr.record_type == RECORD_MARK_PRICE
            && payload.len() >= std::mem::size_of::<MarkPriceRecord>()
        {
            let decoded =
                unsafe { std::ptr::read_unaligned(payload.as_ptr() as *const MarkPriceRecord) };
            shard.update_mark(decoded.symbol_id, decoded.mark_price.0);
        }
    }

    shard.tick(&mut rings, 0);
    assert_eq!(shard.mark_prices[1], 55_000);
}

#[test]
fn bbo_cast_updates_risk_index_price() {
    let mut shard = RiskShard::new(test_config(2));
    let mut rings = make_rings();

    let _recv_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let tmp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let recv_local = tmp.local_addr().unwrap();
    drop(tmp);

    let wal_dir = PathBuf::from("./tmp/cmp_bbo_test");
    let mut sender = CastSender::new(recv_local, 0, &wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver = CastReceiver::new(recv_local, sender_addr).unwrap();

    let mut rec = BboRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: 1,
        _pad0: 0,
        bid_px: rsx_types::Price(99),
        bid_qty: rsx_types::Qty(2),
        bid_count: 1,
        _pad1: 0,
        ask_px: rsx_types::Price(101),
        ask_qty: rsx_types::Qty(1),
        ask_count: 1,
        _pad2: 0,
    };
    sender.send_framed(&Framed::pack(&mut rec, 1)).unwrap();

    thread::sleep(Duration::from_millis(10));
    loop {
        let (hdr, payload) = match receiver.try_recv() {
            CastRecv::Data(h, p) => (h, p),
            CastRecv::Empty => break,
            CastRecv::Faulted { .. } => panic!("unexpected fault"),
            CastRecv::Reconnect { .. } => panic!("unexpected reconnect"),
        };
        if hdr.record_type == RECORD_BBO && payload.len() >= std::mem::size_of::<BboRecord>() {
            let decoded = unsafe { std::ptr::read_unaligned(payload.as_ptr() as *const BboRecord) };
            shard.stash_bbo(BboUpdate {
                seq: decoded.seq,
                symbol_id: decoded.symbol_id,
                bid_px: decoded.bid_px.0,
                bid_qty: decoded.bid_qty.0,
                ask_px: decoded.ask_px.0,
                ask_qty: decoded.ask_qty.0,
            });
        }
    }

    shard.tick(&mut rings, 0);
    assert_eq!(shard.index_prices[1].price, 100);
    assert!(shard.index_prices[1].valid);
}
