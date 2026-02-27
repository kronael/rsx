use rsx_marketdata::config::me_cmp_addrs_from_env;
use rsx_marketdata::config::parse_me_cmp_addrs;

/// Singular RSX_ME_CMP_ADDR value produces exactly one addr.
#[test]
fn singular_addr_parsed() {
    let addrs = parse_me_cmp_addrs("127.0.0.1:9110");
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].port(), 9110);
    assert_eq!(addrs[0].ip().to_string(), "127.0.0.1");
}

/// Multiple comma-separated addresses are all parsed.
#[test]
fn multi_addr_parsed() {
    let addrs =
        parse_me_cmp_addrs("127.0.0.1:9110,127.0.0.1:9103");
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0].port(), 9110);
    assert_eq!(addrs[1].port(), 9103);
}

/// Whitespace around commas is trimmed.
#[test]
fn whitespace_trimmed() {
    let addrs = parse_me_cmp_addrs(
        " 127.0.0.1:9110 , 127.0.0.1:9103 ",
    );
    assert_eq!(addrs.len(), 2);
}

/// Empty string returns empty vec (no silent default).
#[test]
fn empty_string_empty_vec() {
    let addrs = parse_me_cmp_addrs("");
    assert!(addrs.is_empty());
}

/// RSX_ME_CMP_ADDR (singular) is used when ADDRS is absent.
/// Must not silently return the default 127.0.0.1:9100.
#[test]
fn env_singular_addr_no_default() {
    std::env::remove_var("RSX_ME_CMP_ADDRS");
    std::env::set_var("RSX_ME_CMP_ADDR", "127.0.0.1:9110");
    let addrs = me_cmp_addrs_from_env();
    std::env::remove_var("RSX_ME_CMP_ADDR");
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].port(), 9110);
    assert_eq!(addrs[0].ip().to_string(), "127.0.0.1");
}

/// RSX_ME_CMP_ADDRS takes priority over RSX_ME_CMP_ADDR.
#[test]
fn env_addrs_takes_priority() {
    std::env::set_var(
        "RSX_ME_CMP_ADDRS",
        "127.0.0.1:9110,127.0.0.1:9103",
    );
    std::env::set_var("RSX_ME_CMP_ADDR", "127.0.0.1:9101");
    let addrs = me_cmp_addrs_from_env();
    std::env::remove_var("RSX_ME_CMP_ADDRS");
    std::env::remove_var("RSX_ME_CMP_ADDR");
    // ADDRS wins: two entries, not the single from ADDR
    assert_eq!(addrs.len(), 2);
}
