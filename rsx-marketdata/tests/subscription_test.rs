use rsx_marketdata::subscription::*;

#[test]
fn subscribe_adds_symbol_to_client() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 100, CHANNEL_BBO, 10);
    assert!(mgr.has_bbo(1, 100));
    assert_eq!(mgr.client_count(), 1);
}

#[test]
fn subscribe_returns_true_for_new() {
    let mut mgr = SubscriptionManager::new();
    assert!(mgr.subscribe(1, 100, CHANNEL_BBO, 10));
    assert!(!mgr.subscribe(1, 100, CHANNEL_BBO | CHANNEL_DEPTH, 10));
}

#[test]
fn subscribe_multiple_symbols() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 100, CHANNEL_BBO, 10);
    mgr.subscribe(1, 200, CHANNEL_DEPTH, 10);
    assert!(mgr.has_bbo(1, 100));
    assert!(!mgr.has_bbo(1, 200));
    assert!(mgr.has_depth(1, 200));
    assert!(!mgr.has_depth(1, 100));
}

#[test]
fn unsubscribe_removes_symbol() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 100, CHANNEL_BBO, 10);
    mgr.unsubscribe(1, 100);
    assert!(!mgr.has_bbo(1, 100));
    assert!(mgr.clients_for_symbol(100).is_empty());
}

#[test]
fn unsubscribe_all_clears_all() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 100, CHANNEL_BBO, 10);
    mgr.subscribe(1, 200, CHANNEL_DEPTH, 25);
    mgr.unsubscribe_all(1);
    assert!(!mgr.has_bbo(1, 100));
    assert!(!mgr.has_depth(1, 200));
    assert_eq!(mgr.client_count(), 0);
}

#[test]
fn subscribe_with_depth_parameter() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 100, CHANNEL_DEPTH, 25);
    assert_eq!(mgr.client_depth(1), 25);
}

#[test]
fn resubscribe_returns_true_after_unsub() {
    let mut mgr = SubscriptionManager::new();
    assert!(mgr.subscribe(1, 100, CHANNEL_BBO, 10));
    mgr.unsubscribe(1, 100);
    assert!(mgr.subscribe(1, 100, CHANNEL_BBO, 10));
}

#[test]
fn subscribe_depth_10_25_50() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 100, CHANNEL_DEPTH, 10);
    assert_eq!(mgr.client_depth(1), 10);
    mgr.subscribe(1, 100, CHANNEL_DEPTH, 25);
    assert_eq!(mgr.client_depth(1), 25);
    mgr.subscribe(1, 100, CHANNEL_DEPTH, 50);
    assert_eq!(mgr.client_depth(1), 50);
}

#[test]
fn clients_for_symbol_returns_all_subscribers() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(1, 100, CHANNEL_BBO, 10);
    mgr.subscribe(2, 100, CHANNEL_BBO, 10);
    mgr.subscribe(3, 200, CHANNEL_BBO, 10);
    let mut clients = mgr.clients_for_symbol(100);
    clients.sort();
    assert_eq!(clients, vec![1, 2]);
}

#[test]
fn subscribe_bbo_and_depth_channels() {
    let mut mgr = SubscriptionManager::new();
    mgr.subscribe(
        1, 100, CHANNEL_BBO | CHANNEL_DEPTH, 10,
    );
    assert!(mgr.has_bbo(1, 100));
    assert!(mgr.has_depth(1, 100));
}
