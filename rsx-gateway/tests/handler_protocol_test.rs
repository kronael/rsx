//! The connection handler's frame loop: protocol violations close the socket,
//! malformed or unsupported messages get an `E` frame and leave the session
//! open, and a rejected order is never forwarded to risk.

mod common;

use common::cid;
use common::error_frame;
use common::order;
use common::run;
use common::Options;

// ping -> pong carrying the same payload
#[test]
fn ping_is_answered_with_pong() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_frame(true, 0x9, true, b"keepalive").await;
        assert_eq!(client.recv_frame().await, (0xA, b"keepalive".to_vec()));
        assert_eq!(client.settle().await, Vec::<String>::new());
    });
}

// unsolicited pong -> ignored, no reply, session open
#[test]
fn pong_is_ignored() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_frame(true, 0xA, true, b"").await;
        assert_eq!(client.settle().await, Vec::<String>::new());
        assert_eq!(gateway.connection_count(), 1);
    });
}

// client heartbeat -> heartbeat echo
#[test]
fn client_heartbeat_is_echoed() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_text(r#"{"H":[1700000000000]}"#).await;
        let echo = client.recv().await;
        assert!(echo.starts_with(r#"{"H":["#), "echo was {echo}");
    });
}

// close frame -> socket closed, session removed
#[test]
fn close_frame_closes_the_session() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_frame(true, 0x8, true, b"").await;
        assert!(client.read_to_close().await.is_empty());
        assert_eq!(gateway.connection_count(), 0);
    });
}

// text payload of 4097 bytes -> socket closed, no frame
#[test]
fn oversize_frame_closes_the_connection() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_frame(true, 0x1, true, &[b'x'; 4097]).await;
        assert!(client.read_to_close().await.is_empty());
        assert_eq!(gateway.connection_count(), 0);
    });
}

// text payload of exactly 4096 bytes -> read in full, parsed, answered E[1002]
#[test]
fn frame_at_the_size_limit_is_accepted() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let frame = "x".repeat(4096);
        client.reject(&frame, 1002, "invalid json").await;
    });
}

// unmasked client frame -> socket closed, no frame
#[test]
fn unmasked_frame_closes_the_connection() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client
            .send_frame(true, 0x1, false, order(&cid(1)).as_bytes())
            .await;
        assert!(client.read_to_close().await.is_empty());
        gateway.risk.assert_silent();
    });
}

// fragmented frame (FIN=0) -> socket closed, the half message is never parsed
#[test]
fn fragmented_frame_closes_the_connection() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client
            .send_frame(false, 0x1, true, order(&cid(1)).as_bytes())
            .await;
        assert!(client.read_to_close().await.is_empty());
        gateway.risk.assert_silent();
    });
}

// binary frame -> socket closed, no frame
#[test]
fn binary_frame_closes_the_connection() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client
            .send_frame(true, 0x2, true, order(&cid(1)).as_bytes())
            .await;
        assert!(client.read_to_close().await.is_empty());
        gateway.risk.assert_silent();
    });
}

// text that is not UTF-8 -> E[1001], session open
#[test]
fn invalid_utf8_is_rejected_1001() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client
            .send_frame(true, 0x1, true, &[0xFF, 0xFE, 0xFD])
            .await;
        assert_eq!(client.recv().await, error_frame(1001, "invalid utf8"));
        assert_eq!(client.settle().await, Vec::<String>::new());
    });
}

// text that is not valid JSON -> E[1002], session open
#[test]
fn invalid_json_is_rejected_1002() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.reject("not json", 1002, "invalid json").await;
    });
}

// each malformed message -> E[1002] naming the field, session open, nothing forwarded
#[test]
fn malformed_messages_are_rejected_1002() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let cases = [
            (
                format!(r#"{{"N":[0,2,1000,10,"{}",0]}}"#, cid(1)),
                "invalid value: side must be 0 or 1",
            ),
            (
                r#"{"N":[0,0,1000,10,"",0]}"#.to_string(),
                "invalid value: cid must be 1-20 chars",
            ),
            (
                format!(r#"{{"N":[0,0,1000,10,"{}x",0]}}"#, cid(1)),
                "invalid value: cid must be 1-20 chars",
            ),
            (
                format!(r#"{{"N":[0,0,1000,10,"{}",3]}}"#, cid(1)),
                "invalid value: tif must be 0, 1, or 2",
            ),
            (
                r#"{"N":[0,0,1000]}"#.to_string(),
                "missing field: N requires at least 6 fields",
            ),
            (r#"{"N":[],"C":[]}"#.to_string(), "multiple keys"),
            (r#"{"Z":[]}"#.to_string(), "unknown type: Z"),
        ];
        for (frame, message) in cases {
            client.reject(&frame, 1002, message).await;
        }
        gateway.risk.assert_silent();
    });
}

// server-to-client message types sent by the client -> E[1004], session open
#[test]
fn server_message_types_are_unsupported_1004() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let oid = "0".repeat(32);
        let cases = [
            format!(r#"{{"U":["{oid}",1,0,10,0]}}"#),
            format!(r#"{{"F":["{oid}","{oid}",1000,10,1,0]}}"#),
            r#"{"E":[1,"x"]}"#.to_string(),
            r#"{"Q":[0,1,2,1,30,900,15]}"#.to_string(),
        ];
        for frame in cases {
            client.reject(&frame, 1004, "unsupported").await;
        }
    });
}

// order for a symbol the gateway does not know -> E[1007], not forwarded
#[test]
fn unknown_symbol_is_rejected_1007() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        let frame = format!(r#"{{"N":[2,0,1000,10,"{}",0]}}"#, cid(1));
        client.reject(&frame, 1007, "unknown symbol").await;
        gateway.risk.assert_silent();
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}

// price off the tick (tick 10), zero or negative -> E[1008], not forwarded
#[test]
fn bad_price_is_rejected_1008() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        for price in [1005, 0, -10] {
            let frame = format!(r#"{{"N":[0,0,{price},10,"{}",0]}}"#, cid(1));
            client.reject(&frame, 1008, "price not tick aligned").await;
        }
        gateway.risk.assert_silent();
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}

// qty off the lot (lot 5), zero or negative -> E[1009], not forwarded
#[test]
fn bad_qty_is_rejected_1009() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        for qty in [7, 0, -5] {
            let frame = format!(r#"{{"N":[0,0,1000,{qty},"{}",0]}}"#, cid(1));
            client.reject(&frame, 1009, "qty not lot aligned").await;
        }
        gateway.risk.assert_silent();
        assert_eq!(gateway.state.borrow().pending.len(), 0);
    });
}
