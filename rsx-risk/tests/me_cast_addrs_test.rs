use rsx_risk::me_cast_addrs_from_env;
use rsx_risk::parse_me_cast_addrs;
use rsx_risk::parse_me_streams;
use std::sync::Mutex;

// Env is process-global; the env_* tests mutate RSX_ME_CAST_ADDR(S) and run
// on parallel threads, so serialize them. Poison-tolerant: a panicking test
// still releases the env to the next.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Singular RSX_ME_CAST_ADDR produces exactly one entry with
/// the correct symbol_id derived from port - 9100.
#[test]
fn singular_addr_parsed() {
    let map = parse_me_cast_addrs("127.0.0.1:9110");
    assert_eq!(map.len(), 1);
    let addr = map.get(&10).expect("symbol_id 10 not found");
    assert_eq!(addr.port(), 9110);
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
}

/// Multiple comma-separated addresses all parsed correctly.
#[test]
fn multi_addr_parsed() {
    let map = parse_me_cast_addrs("127.0.0.1:9110,127.0.0.1:9103");
    assert_eq!(map.len(), 2);
    assert!(map.contains_key(&10)); // PENGU id=10
    assert!(map.contains_key(&3)); // SOL id=3
}

/// Spaces around commas are trimmed.
#[test]
fn whitespace_trimmed() {
    let map = parse_me_cast_addrs(" 127.0.0.1:9110 , 127.0.0.1:9103 ");
    assert_eq!(map.len(), 2);
}

/// Invalid entries are skipped, valid ones still parsed.
#[test]
fn invalid_entry_skipped() {
    let map = parse_me_cast_addrs("127.0.0.1:9110,not-an-addr");
    assert_eq!(map.len(), 1);
    assert!(map.contains_key(&10));
}

/// Empty string produces empty map (no silent default).
#[test]
fn empty_string_empty_map() {
    let map = parse_me_cast_addrs("");
    assert!(map.is_empty());
}

/// RSX_ME_CAST_ADDR (singular) is used when ADDRS is absent.
/// Must not silently fall back to 127.0.0.1:9110 default.
#[test]
fn env_singular_addr_no_default() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("RSX_ME_CAST_ADDRS");
    std::env::set_var("RSX_ME_CAST_ADDR", "127.0.0.1:9103");
    let map = me_cast_addrs_from_env();
    std::env::remove_var("RSX_ME_CAST_ADDR");
    assert_eq!(map.len(), 1);
    let addr = map.get(&3).expect("symbol_id 3 not found");
    assert_eq!(addr.port(), 9103);
}

/// RSX_ME_CAST_ADDRS takes priority over RSX_ME_CAST_ADDR.
#[test]
fn env_addrs_takes_priority() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("RSX_ME_CAST_ADDRS", "127.0.0.1:9110");
    std::env::set_var("RSX_ME_CAST_ADDR", "127.0.0.1:9103");
    let map = me_cast_addrs_from_env();
    std::env::remove_var("RSX_ME_CAST_ADDRS");
    std::env::remove_var("RSX_ME_CAST_ADDR");
    // ADDRS wins: only PENGU (id=10) present
    assert_eq!(map.len(), 1);
    assert!(map.contains_key(&10));
}

#[test]
fn multi_streams_keep_independent_endpoints() {
    let streams = parse_me_streams(
        parse_me_cast_addrs("127.0.0.1:9110,127.0.0.1:9103"),
        Some("10=127.0.0.1:28310,3=127.0.0.1:28303"),
        Some("10=127.0.0.1:9710,3=127.0.0.1:9703"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(streams[0].symbol_id, 3);
    assert_eq!(streams[0].recv_addr.port(), 28303);
    assert_eq!(streams[0].replay_addr, "127.0.0.1:9703");
    assert_eq!(streams[1].symbol_id, 10);
    assert_eq!(streams[1].recv_addr.port(), 28310);
    assert_eq!(streams[1].replay_addr, "127.0.0.1:9710");
}

#[test]
fn multi_streams_reject_missing_or_shared_peers() {
    let casts = parse_me_cast_addrs("127.0.0.1:9110,127.0.0.1:9103");
    assert!(parse_me_streams(
        casts.clone(),
        None,
        None,
        Some("127.0.0.1:28301".parse().unwrap()),
        Some("127.0.0.1:9710"),
    )
    .is_err());
    assert!(parse_me_streams(
        casts,
        Some("10=127.0.0.1:28301,3=127.0.0.1:28301"),
        Some("10=127.0.0.1:9710,3=127.0.0.1:9703"),
        None,
        None,
    )
    .is_err());
}

#[test]
fn singular_stream_remains_supported() {
    let streams = parse_me_streams(
        parse_me_cast_addrs("127.0.0.1:9110"),
        None,
        None,
        Some("127.0.0.1:28310".parse().unwrap()),
        Some("127.0.0.1:9710"),
    )
    .unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].symbol_id, 10);
}
