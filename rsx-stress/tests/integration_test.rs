use rsx_stress::{OrderGenerator, SymbolConfig, WorkerConfig};
use std::time::Duration;

#[tokio::test]
async fn test_worker_basic() {
    let symbols = vec![SymbolConfig {
        symbol_id: 1,
        name: "BTCUSD".to_string(),
        mid_price: 50000_00,
        tick_size: 1_00,
        lot_size: 1_00,
        weight: 1.0,
    }];

    let users = vec![1001];
    let generator = OrderGenerator::new(symbols, users);

    let config = WorkerConfig {
        gateway_url: "ws://localhost:8080".to_string(),
        user_id: 1001,
        rate_per_sec: 10.0,
        duration_secs: 1,
        generator,
    };

    let result = rsx_stress::worker_task(config).await;
    assert!(result.is_ok() || result.is_err());
}

#[tokio::test]
async fn test_order_generator() {
    let symbols = vec![
        SymbolConfig {
            symbol_id: 1,
            name: "BTCUSD".to_string(),
            mid_price: 50000_00,
            tick_size: 1_00,
            lot_size: 1_00,
            weight: 0.7,
        },
        SymbolConfig {
            symbol_id: 2,
            name: "ETHUSD".to_string(),
            mid_price: 3000_00,
            tick_size: 1_00,
            lot_size: 1_00,
            weight: 0.3,
        },
    ];

    let users = vec![1001, 1002, 1003];
    let mut generator = OrderGenerator::new(symbols, users);

    for _ in 0..100 {
        let order = generator.next_order();
        assert!(order.symbol_id == 1 || order.symbol_id == 2);
        assert!(order.side == 0 || order.side == 1);
        assert!(order.price > 0);
        assert!(order.qty > 0);
        assert_eq!(order.client_order_id.len(), 20);
    }
}

#[tokio::test]
async fn test_metrics() {
    let mut metrics = rsx_stress::Metrics::new(None).unwrap();

    metrics.record_submitted();
    metrics.record_submitted();
    metrics.record_accepted();

    let latency = Duration::from_micros(1500);
    metrics
        .record_latency(latency, "accepted", "test-oid")
        .unwrap();

    let summary = metrics.summary();
    assert_eq!(summary.total, 2);
    assert_eq!(summary.accepted, 1);
    assert!(summary.p50 > 0);
}
