use rsx_risk::parse_me_cmp_addrs;

/// Singular RSX_ME_CMP_ADDR produces exactly one entry with
/// the correct symbol_id derived from port - 9100.
#[test]
fn singular_addr_parsed() {
    let map = parse_me_cmp_addrs("127.0.0.1:9110");
    assert_eq!(map.len(), 1);
    let addr = map.get(&10).expect("symbol_id 10 not found");
    assert_eq!(addr.port(), 9110);
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
}

/// Multiple comma-separated addresses all parsed correctly.
#[test]
fn multi_addr_parsed() {
    let map =
        parse_me_cmp_addrs("127.0.0.1:9110,127.0.0.1:9103");
    assert_eq!(map.len(), 2);
    assert!(map.contains_key(&10)); // PENGU id=10
    assert!(map.contains_key(&3)); // SOL id=3
}

/// Spaces around commas are trimmed.
#[test]
fn whitespace_trimmed() {
    let map =
        parse_me_cmp_addrs(" 127.0.0.1:9110 , 127.0.0.1:9103 ");
    assert_eq!(map.len(), 2);
}

/// Invalid entries are skipped, valid ones still parsed.
#[test]
fn invalid_entry_skipped() {
    let map =
        parse_me_cmp_addrs("127.0.0.1:9110,not-an-addr");
    assert_eq!(map.len(), 1);
    assert!(map.contains_key(&10));
}

/// Empty string produces empty map (no silent default).
#[test]
fn empty_string_empty_map() {
    let map = parse_me_cmp_addrs("");
    assert!(map.is_empty());
}
