use rsx_messages::*;

fn cid20() -> String {
    "abcdefghij0123456789".to_string()
}

fn oid32() -> String {
    "0123456789abcdef0123456789abcdef".to_string()
}

// --- Parsing tests ---

#[test]
fn parse_n_frame_all_fields() {
    let json = format!("{{\"N\":[1,0,50000,100,\"{}\",0,1,1]}}", cid20());
    let f = parse(&json).unwrap();
    assert_eq!(
        f,
        WsFrame::NewOrder {
            symbol_id: 1,
            side: 0,
            price: 50000,
            qty: 100,
            client_order_id: cid20(),
            tif: 0,
            reduce_only: true,
            post_only: true,
        }
    );
}

#[test]
fn parse_n_frame_reduce_only_default_0() {
    let json = format!("{{\"N\":[1,1,50000,100,\"{}\",2]}}", cid20());
    let f = parse(&json).unwrap();
    match f {
        WsFrame::NewOrder {
            reduce_only,
            post_only,
            tif,
            side,
            ..
        } => {
            assert!(!reduce_only);
            assert!(!post_only);
            assert_eq!(tif, 2);
            assert_eq!(side, 1);
        }
        _ => panic!("expected NewOrder"),
    }
}

#[test]
fn parse_n_frame_reduce_only_1() {
    let json = format!("{{\"N\":[1,0,50000,100,\"{}\",0,1,0]}}", cid20());
    match parse(&json).unwrap() {
        WsFrame::NewOrder { reduce_only, .. } => {
            assert!(reduce_only);
        }
        _ => panic!("expected NewOrder"),
    }
}

#[test]
fn parse_n_frame_post_only_1() {
    let json = format!("{{\"N\":[1,0,50000,100,\"{}\",0,0,1]}}", cid20());
    match parse(&json).unwrap() {
        WsFrame::NewOrder { post_only, .. } => {
            assert!(post_only);
        }
        _ => panic!("expected NewOrder"),
    }
}

#[test]
fn parse_n_frame_invalid_side_rejected() {
    let json = format!("{{\"N\":[1,2,50000,100,\"{}\",0]}}", cid20());
    assert!(matches!(parse(&json), Err(ParseError::InvalidValue(_))));
}

#[test]
fn parse_n_frame_symbol_id_over_u32_rejected() {
    // u32::MAX + 1 must not truncate to 0 (wrong routing).
    let json = format!("{{\"N\":[4294967296,0,50000,100,\"{}\",0]}}", cid20());
    assert!(matches!(parse(&json), Err(ParseError::InvalidValue(_))));
}

#[test]
fn parse_n_frame_symbol_id_u32_max_ok() {
    let json = format!("{{\"N\":[4294967295,0,50000,100,\"{}\",0]}}", cid20());
    match parse(&json).unwrap() {
        WsFrame::NewOrder { symbol_id, .. } => {
            assert_eq!(symbol_id, u32::MAX);
        }
        other => panic!("expected NewOrder, got {other:?}"),
    }
}

#[test]
fn parse_n_frame_missing_field_rejected() {
    let json = r#"{"N":[1,0,50000]}"#;
    assert!(matches!(parse(json), Err(ParseError::MissingField(_))));
}

#[test]
fn parse_c_frame_by_cid() {
    let json = format!("{{\"C\":[\"{}\"]}}", cid20());
    let f = parse(&json).unwrap();
    assert_eq!(
        f,
        WsFrame::Cancel {
            key: CancelKey::ClientOrderId(cid20()),
        }
    );
}

#[test]
fn parse_c_frame_by_oid() {
    let json = format!("{{\"C\":[\"{}\"]}}", oid32());
    let f = parse(&json).unwrap();
    assert_eq!(
        f,
        WsFrame::Cancel {
            key: CancelKey::OrderId(oid32()),
        }
    );
}

#[test]
fn parse_c_frame_by_short_cid() {
    for cid in ["a", "short-cid-15byt"] {
        let f = parse(&format!("{{\"C\":[\"{cid}\"]}}")).unwrap();
        assert_eq!(
            f,
            WsFrame::Cancel {
                key: CancelKey::ClientOrderId(cid.to_string()),
            }
        );
    }
}

#[test]
fn parse_c_frame_rejects_a_key_of_neither_length() {
    for key in ["", &"a".repeat(21), &"a".repeat(31), &"a".repeat(33)] {
        let json = format!("{{\"C\":[\"{key}\"]}}");
        assert!(
            matches!(parse(&json), Err(ParseError::InvalidValue(_))),
            "key length {}",
            key.len()
        );
    }
}

#[test]
fn parse_h_frame_server_initiated() {
    let json = r#"{"H":[1700000000000]}"#;
    let f = parse(json).unwrap();
    assert_eq!(
        f,
        WsFrame::Heartbeat {
            timestamp_ms: 1700000000000,
        }
    );
}

#[test]
fn parse_h_frame_client_echo() {
    let json = r#"{"H":[9999]}"#;
    let f = parse(json).unwrap();
    assert_eq!(f, WsFrame::Heartbeat { timestamp_ms: 9999 });
}

#[test]
fn parse_e_frame_error_code_and_msg() {
    let json = r#"{"E":[1001,"rate limited"]}"#;
    let f = parse(json).unwrap();
    assert_eq!(
        f,
        WsFrame::Error {
            code: 1001,
            message: "rate limited".to_string(),
        }
    );
}

#[test]
fn parse_q_frame_liquidation_all_statuses() {
    for status in 0..=4u8 {
        let json = format!("{{\"Q\":[1,{},10,0,500,49000,50]}}", status,);
        let f = parse(&json).unwrap();
        match f {
            WsFrame::Liquidation { status: s, .. } => {
                assert_eq!(s, status);
            }
            _ => panic!("expected Liquidation"),
        }
    }
    // status=5 rejected
    let json = r#"{"Q":[1,5,10,0,500,49000,50]}"#;
    assert!(parse(json).is_err());
}

#[test]
fn parse_frame_rejects_multiple_keys() {
    let json = r#"{"N":[],"C":[]}"#;
    assert!(matches!(parse(json), Err(ParseError::MultipleKeys)));
}

#[test]
fn parse_frame_rejects_non_letter_key() {
    let json = r#"{"1":[123]}"#;
    assert!(matches!(parse(json), Err(ParseError::UnknownType(_))));
}

#[test]
fn parse_n_frame_invalid_tif_rejected() {
    let json = format!("{{\"N\":[1,0,50000,100,\"{}\",5]}}", cid20());
    assert!(matches!(parse(&json), Err(ParseError::InvalidValue(_))));
}

// --- Serialization tests ---

#[test]
fn serialize_u_frame_order_update() {
    let f = WsFrame::OrderUpdate {
        order_id: oid32(),
        status: 0,
        filled_qty: 100,
        remaining_qty: 0,
        reason: 0,
        gw_in_ns: 1,
        risk_in_ns: 0,
        me_in_ns: 2,
        match_done_ns: 3,
        gw_out_ns: 4,
    };
    let s = serialize(&f);
    let parsed = parse(&s).unwrap();
    assert_eq!(parsed, f);
}

#[test]
fn serialize_f_frame_fill() {
    let f = WsFrame::Fill {
        taker_order_id: oid32(),
        maker_order_id: oid32(),
        price: 50000,
        qty: 100,
        timestamp_ns: 1700000,
        fee: 25,
        gw_in_ns: 1,
        risk_in_ns: 2,
        me_in_ns: 3,
        match_done_ns: 4,
        gw_out_ns: 5,
    };
    let s = serialize(&f);
    let parsed = parse(&s).unwrap();
    assert_eq!(parsed, f);
}

#[test]
fn serialize_e_frame_error() {
    let f = WsFrame::Error {
        code: 1001,
        message: "bad request".to_string(),
    };
    let s = serialize(&f);
    let parsed = parse(&s).unwrap();
    assert_eq!(parsed, f);
}

#[test]
fn serialize_h_frame_heartbeat() {
    let f = WsFrame::Heartbeat {
        timestamp_ms: 1700000000000,
    };
    let s = serialize(&f);
    let parsed = parse(&s).unwrap();
    assert_eq!(parsed, f);
}

#[test]
fn serialize_q_frame_liquidation() {
    let f = WsFrame::Liquidation {
        symbol_id: 1,
        status: 2,
        round: 10,
        side: 0,
        qty: 500,
        price: 49000,
        slip_bps: 50,
    };
    let s = serialize(&f);
    let parsed = parse(&s).unwrap();
    assert_eq!(parsed, f);
}

// --- Enum validation ---

#[test]
fn enum_side_valid_0_1_only() {
    for side in 0..=1u8 {
        let json = format!("{{\"N\":[1,{},50000,100,\"{}\",0]}}", side, cid20(),);
        assert!(parse(&json).is_ok());
    }
    let json = format!("{{\"N\":[1,2,50000,100,\"{}\",0]}}", cid20(),);
    assert!(parse(&json).is_err());
}

#[test]
fn enum_tif_valid_0_1_2_only() {
    for tif in 0..=2u8 {
        let json = format!("{{\"N\":[1,0,50000,100,\"{}\",{}]}}", cid20(), tif,);
        assert!(parse(&json).is_ok());
    }
    let json = format!("{{\"N\":[1,0,50000,100,\"{}\",3]}}", cid20(),);
    assert!(parse(&json).is_err());
}

#[test]
fn enum_order_status_valid_0_1_2_3() {
    for st in 0..=3u8 {
        let json = format!("{{\"U\":[\"{}\",{},100,0,0]}}", oid32(), st,);
        assert!(parse(&json).is_ok());
    }
    let json = format!("{{\"U\":[\"{}\",4,100,0,0]}}", oid32(),);
    assert!(parse(&json).is_err());
}

#[test]
fn enum_failure_reason_valid_0_through_14() {
    for r in 0..=14u8 {
        let json = format!("{{\"U\":[\"{}\",3,0,100,{}]}}", oid32(), r,);
        assert!(parse(&json).is_ok());
    }
    let json = format!("{{\"U\":[\"{}\",3,0,100,15]}}", oid32(),);
    assert!(parse(&json).is_err());
}

// --- Unknown enum ---

#[test]
fn enum_unknown_value_rejected() {
    // Unknown message type
    let json = r#"{"Z":[1,2,3]}"#;
    assert!(matches!(parse(json), Err(ParseError::UnknownType(_))));
}

#[test]
fn n_frame_ro_1_maps_to_quic_reduce_only() {
    let json = format!("{{\"N\":[1,0,50000,100,\"{}\",0,1]}}", cid20());
    match parse(&json).unwrap() {
        WsFrame::NewOrder { reduce_only, .. } => {
            assert!(reduce_only);
        }
        _ => panic!("expected NewOrder"),
    }
}
