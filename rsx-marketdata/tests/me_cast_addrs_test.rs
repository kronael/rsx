use rsx_marketdata::config::me_cast_addrs_from_env;
use rsx_marketdata::config::parse_me_cast_addrs;
use rsx_marketdata::config::parse_me_sender_addrs;
use std::sync::Mutex;

// Env is process-global; the env_* tests mutate RSX_ME_CAST_ADDR(S) and run
// on parallel threads, so serialize them. Poison-tolerant: a panicking test
// still releases the env to the next.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Singular RSX_ME_CAST_ADDR value produces exactly one addr.
#[test]
fn singular_addr_parsed() {
    let addrs = parse_me_cast_addrs("127.0.0.1:9110");
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].port(), 9110);
    assert_eq!(addrs[0].ip().to_string(), "127.0.0.1");
}

/// Multiple comma-separated addresses are all parsed.
#[test]
fn multi_addr_parsed() {
    let addrs = parse_me_cast_addrs("127.0.0.1:9110,127.0.0.1:9103");
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0].port(), 9110);
    assert_eq!(addrs[1].port(), 9103);
}

/// Whitespace around commas is trimmed.
#[test]
fn whitespace_trimmed() {
    let addrs = parse_me_cast_addrs(" 127.0.0.1:9110 , 127.0.0.1:9103 ");
    assert_eq!(addrs.len(), 2);
}

/// Empty string returns empty vec (no silent default).
#[test]
fn empty_string_empty_vec() {
    let addrs = parse_me_cast_addrs("");
    assert!(addrs.is_empty());
}

/// RSX_ME_CAST_ADDR (singular) is used when ADDRS is absent.
/// Must not silently return the default 127.0.0.1:9100.
#[test]
fn env_singular_addr_no_default() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("RSX_ME_CAST_ADDRS");
    std::env::set_var("RSX_ME_CAST_ADDR", "127.0.0.1:9110");
    let addrs = me_cast_addrs_from_env();
    std::env::remove_var("RSX_ME_CAST_ADDR");
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].port(), 9110);
    assert_eq!(addrs[0].ip().to_string(), "127.0.0.1");
}

/// RSX_ME_CAST_ADDRS takes priority over RSX_ME_CAST_ADDR.
#[test]
fn env_addrs_takes_priority() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("RSX_ME_CAST_ADDRS", "127.0.0.1:9110,127.0.0.1:9103");
    std::env::set_var("RSX_ME_CAST_ADDR", "127.0.0.1:9101");
    let addrs = me_cast_addrs_from_env();
    std::env::remove_var("RSX_ME_CAST_ADDRS");
    std::env::remove_var("RSX_ME_CAST_ADDR");
    // ADDRS wins: two entries, not the single from ADDR
    assert_eq!(addrs.len(), 2);
}

/// Without `RSX_ME_MD_SEND_ADDRS` each receiver NAKs the ME cast address.
#[test]
fn sender_addrs_default_to_the_me_cast_addrs() {
    let me_addrs = parse_me_cast_addrs("127.0.0.1:9110,127.0.0.1:9111");
    let senders = parse_me_sender_addrs(None, &me_addrs).unwrap();
    assert_eq!(senders, me_addrs);
}

/// `RSX_ME_MD_SEND_ADDRS` is positional: the nth sender serves the nth ME.
#[test]
fn sender_addrs_follow_the_me_order() {
    let me_addrs = parse_me_cast_addrs("127.0.0.1:9110,127.0.0.1:9111");
    let senders =
        parse_me_sender_addrs(Some("127.0.0.1:11410, 127.0.0.1:11411"), &me_addrs).unwrap();
    assert_eq!(senders[0].port(), 11410);
    assert_eq!(senders[1].port(), 11411);
}

/// A count that differs from the ME count, or a malformed address, is an error.
#[test]
fn sender_addrs_reject_a_wrong_count_and_a_bad_address() {
    let me_addrs = parse_me_cast_addrs("127.0.0.1:9110,127.0.0.1:9111");
    assert!(parse_me_sender_addrs(Some("127.0.0.1:11410"), &me_addrs).is_err());
    assert!(parse_me_sender_addrs(Some("127.0.0.1:11410,nope"), &me_addrs).is_err());
}
