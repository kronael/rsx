use rsx_gateway::order_id::generate_order_id;
use rsx_gateway::order_id::hex_to_order_id;
use rsx_gateway::order_id::order_id_timestamp_ms;
use rsx_gateway::order_id::order_id_to_hex;

#[test]
fn uuid_v7_16_bytes_binary() {
    let id = generate_order_id();
    assert_eq!(id.len(), 16);
}

#[test]
fn uuid_v7_monotonic_within_millisecond() {
    let mut prev = generate_order_id();
    for _ in 0..100 {
        let curr = generate_order_id();
        assert!(curr >= prev, "not monotonic");
        prev = curr;
    }
}

#[test]
fn uuid_v7_globally_unique_across_instances() {
    let mut ids = std::collections::HashSet::new();
    for _ in 0..10_000 {
        let id = generate_order_id();
        assert!(ids.insert(id), "duplicate id");
    }
}

#[test]
fn uuid_v7_time_sortable() {
    let id1 = generate_order_id();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let id2 = generate_order_id();
    let ts1 = order_id_timestamp_ms(&id1);
    let ts2 = order_id_timestamp_ms(&id2);
    assert!(ts2 > ts1, "not time-sortable");
}

#[test]
fn order_id_hex_roundtrip() {
    let id = generate_order_id();
    let hex = order_id_to_hex(&id);
    assert_eq!(hex.len(), 32);
    let parsed = hex_to_order_id(&hex).unwrap();
    assert_eq!(id, parsed);
}

#[test]
fn hex_to_order_id_invalid_length() {
    assert!(hex_to_order_id("abc").is_none());
    assert!(hex_to_order_id("").is_none());
}

#[test]
fn hex_to_order_id_invalid_chars() {
    let bad = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
    assert!(hex_to_order_id(bad).is_none());
}

#[test]
fn hex_to_order_id_rejects_non_ascii_and_signs() {
    let multibyte = format!("a{}b", "\u{e9}".repeat(15));
    assert_eq!(multibyte.len(), 32);
    assert!(hex_to_order_id(&multibyte).is_none());
    assert!(hex_to_order_id(&"+f".repeat(16)).is_none());
    assert_eq!(hex_to_order_id(&"Ff".repeat(16)), Some([0xff; 16]));
}

#[test]
fn order_id_timestamp_reasonable() {
    let id = generate_order_id();
    let ts = order_id_timestamp_ms(&id);
    assert!(ts > 1_704_067_200_000);
    assert!(ts < 1_893_456_000_000);
}
