use rsx_dxs::cmp::CmpReceiver;
use rsx_dxs::cmp::CmpSender;
use rsx_dxs::records::BboRecord;
use rsx_dxs::records::MarkPriceRecord;
use rsx_dxs::records::RECORD_BBO;
use rsx_dxs::records::RECORD_MARK_PRICE;
use rsx_risk::config::LiquidationConfig;
use rsx_risk::config::ReplicationConfig;
use rsx_risk::config::ShardConfig;
use rsx_risk::funding::FundingConfig;
use rsx_risk::margin::SymbolRiskParams;
use rsx_risk::rings::MarkPriceUpdate;
use rsx_risk::rings::OrderResponse;
use rsx_risk::rings::ShardRings;
use rsx_risk::shard::RiskShard;
use rsx_risk::types::BboUpdate;
use rsx_risk::types::FillEvent;
use rsx_risk::types::OrderRequest;
use rtrb::Producer;
use rtrb::RingBuffer;
use std::net::SocketAddr;
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
            is_replica: false,
            lease_poll_interval_ms: 500,
            lease_renew_interval_ms: 1000,
            replica_sync_ring_size: 1024,
        },
    }
}

fn make_rings(
) -> (ShardRings, Producer<MarkPriceUpdate>, Producer<BboUpdate>) {
    let (_fill_prod, fill_cons) =
        RingBuffer::<FillEvent>::new(8);
    let (_order_prod, order_cons) =
        RingBuffer::<OrderRequest>::new(8);
    let (mark_prod, mark_cons) =
        RingBuffer::<MarkPriceUpdate>::new(8);
    let (bbo_prod, bbo_cons) =
        RingBuffer::<BboUpdate>::new(8);
    let (resp_prod, _resp_cons) =
        RingBuffer::<OrderResponse>::new(8);
    let (acc_prod, _acc_cons) =
        RingBuffer::<OrderRequest>::new(8);

    let rings = ShardRings {
        fill_consumers: vec![fill_cons],
        order_consumer: order_cons,
        mark_consumer: mark_cons,
        bbo_consumers: vec![bbo_cons],
        response_producer: resp_prod,
        accepted_producer: acc_prod,
    };

    (rings, mark_prod, bbo_prod)
}

#[test]
fn mark_cmp_updates_risk_mark_prices() {
    let mut shard = RiskShard::new(test_config(2));
    let (mut rings, mut mark_prod, _bbo_prod) = make_rings();

    let recv_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let tmp = std::net::UdpSocket::bind(recv_addr).unwrap();
    let recv_local = tmp.local_addr().unwrap();
    drop(tmp);

    let wal_dir = PathBuf::from("./tmp/cmp_mark_test");
    let mut sender = CmpSender::new(recv_local, 0, &wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver = CmpReceiver::new(recv_local, sender_addr, 0).unwrap();

    let mut rec = MarkPriceRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: 1,
        _pad0: 0,
        mark_price: 55_000,
        source_mask: 0,
        source_count: 1,
        _pad1: [0; 24],
    };
    sender.send(&mut rec).unwrap();

    thread::sleep(Duration::from_millis(10));
    while let Some((hdr, payload)) = receiver.try_recv() {
        if hdr.record_type == RECORD_MARK_PRICE
            && payload.len()
                >= std::mem::size_of::<MarkPriceRecord>()
        {
            let decoded = unsafe {
                std::ptr::read_unaligned(
                    payload.as_ptr() as *const MarkPriceRecord,
                )
            };
            let _ = mark_prod.push(MarkPriceUpdate {
                seq: decoded.seq,
                symbol_id: decoded.symbol_id,
                price: decoded.mark_price,
            });
        }
    }

    shard.run_once(&mut rings, 0);
    assert_eq!(shard.mark_prices[1], 55_000);
}

#[test]
fn bbo_cmp_updates_risk_index_price() {
    let mut shard = RiskShard::new(test_config(2));
    let (mut rings, _mark_prod, mut bbo_prod) = make_rings();

    let recv_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let tmp = std::net::UdpSocket::bind(recv_addr).unwrap();
    let recv_local = tmp.local_addr().unwrap();
    drop(tmp);

    let wal_dir = PathBuf::from("./tmp/cmp_bbo_test");
    let mut sender = CmpSender::new(recv_local, 0, &wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();
    let mut receiver = CmpReceiver::new(recv_local, sender_addr, 0).unwrap();

    let mut rec = BboRecord {
        seq: 0,
        ts_ns: 1,
        symbol_id: 1,
        _pad0: 0,
        bid_px: 99,
        bid_qty: 2,
        bid_count: 1,
        _pad1: 0,
        ask_px: 101,
        ask_qty: 1,
        ask_count: 1,
        _pad2: 0,
    };
    sender.send(&mut rec).unwrap();

    thread::sleep(Duration::from_millis(10));
    while let Some((hdr, payload)) = receiver.try_recv() {
        if hdr.record_type == RECORD_BBO
            && payload.len()
                >= std::mem::size_of::<BboRecord>()
        {
            let decoded = unsafe {
                std::ptr::read_unaligned(
                    payload.as_ptr() as *const BboRecord,
                )
            };
            let _ = bbo_prod.push(BboUpdate {
                seq: decoded.seq,
                symbol_id: decoded.symbol_id,
                bid_px: decoded.bid_px,
                bid_qty: decoded.bid_qty,
                ask_px: decoded.ask_px,
                ask_qty: decoded.ask_qty,
            });
        }
    }

    shard.run_once(&mut rings, 0);
    assert_eq!(shard.index_prices[1].price, 100);
    assert!(shard.index_prices[1].valid);
}
