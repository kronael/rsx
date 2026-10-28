//! The connection handler's session lifecycle: a valid token opens a session
//! that greets with a heartbeat, any other token or none gets a 401 and a
//! closed socket, a user's sixth connection is closed after the 101, and a
//! closed socket ends the session. The gateway closes a session by removing it
//! from the state (idle reap, full outbound queue) and sends heartbeats from
//! its housekeeping loop.

mod common;

use common::cid;
use common::mint_token;
use common::order;
use common::run;
use common::token;
use common::Options;
use common::AUDIENCE;
use common::ISSUER;
use common::SECRET;
use monoio::io::AsyncReadRent;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpStream;
use rsx_ws::WS_UPGRADE_DEADLINE;
use std::time::Duration;

// valid token -> 101 with the RFC 6455 accept key, greeting heartbeat, session registered
#[test]
fn valid_token_upgrades_and_greets_with_heartbeat() {
    run(Options::default(), |gateway| async move {
        let (head, mut client) = gateway.upgrade(Some(&token(7))).await;
        assert!(head.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        assert!(head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
        let greeting = client.recv().await;
        assert!(greeting.starts_with(r#"{"H":["#), "greeting was {greeting}");
        assert_eq!(gateway.connection_count(), 1);
        let user_id = gateway.state.borrow().connections[&0].user_id;
        assert_eq!(user_id, 7);
    });
}

// same token twice -> two 101s and two live sessions, a JWT is reusable until exp
#[test]
fn same_token_opens_two_connections() {
    run(Options::default(), |gateway| async move {
        let reused = token(7);
        let (first, _first_client) = gateway.upgrade(Some(&reused)).await;
        let (second, _second_client) = gateway.upgrade(Some(&reused)).await;
        assert!(first.starts_with("HTTP/1.1 101"), "first got {first}");
        assert!(second.starts_with("HTTP/1.1 101"), "second got {second}");
        gateway
            .wait_until("both sessions registered", |gateway| {
                gateway.connection_count() == 2
            })
            .await;
    });
}

// upgrade without an Authorization header -> 401, nothing else, socket closed
#[test]
fn missing_authorization_header_gets_401_and_close() {
    run(Options::default(), |gateway| async move {
        let (head, mut client) = gateway.upgrade(None).await;
        assert_eq!(
            head,
            "HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n"
        );
        assert!(client.read_to_close().await.is_empty());
        assert_eq!(gateway.connection_count(), 0);
    });
}

// token for another audience -> 401, socket closed, no session
#[test]
fn wrong_audience_gets_401() {
    run(Options::default(), |gateway| async move {
        let wrong = mint_token(7, "rsx-marketdata", ISSUER, SECRET);
        let (head, mut client) = gateway.upgrade(Some(&wrong)).await;
        assert!(head.starts_with("HTTP/1.1 401 Unauthorized"), "got {head}");
        assert!(client.read_to_close().await.is_empty());
        assert_eq!(gateway.connection_count(), 0);
    });
}

// token from another issuer -> 401, socket closed, no session
#[test]
fn wrong_issuer_gets_401() {
    run(Options::default(), |gateway| async move {
        let wrong = mint_token(7, AUDIENCE, "someone-else", SECRET);
        let (head, mut client) = gateway.upgrade(Some(&wrong)).await;
        assert!(head.starts_with("HTTP/1.1 401 Unauthorized"), "got {head}");
        assert!(client.read_to_close().await.is_empty());
        assert_eq!(gateway.connection_count(), 0);
    });
}

// token signed with another secret -> 401, socket closed, no session
#[test]
fn wrong_signature_gets_401() {
    run(Options::default(), |gateway| async move {
        let wrong = mint_token(7, AUDIENCE, ISSUER, "another-secret-padded-to-32-bytes!!");
        let (head, mut client) = gateway.upgrade(Some(&wrong)).await;
        assert!(head.starts_with("HTTP/1.1 401 Unauthorized"), "got {head}");
        assert!(client.read_to_close().await.is_empty());
        assert_eq!(gateway.connection_count(), 0);
    });
}

// Authorization header that is not a Bearer token -> 401
#[test]
fn non_bearer_authorization_gets_401() {
    run(Options::default(), |gateway| async move {
        let (head, mut client) = gateway
            .upgrade_with_header("Authorization: Basic dXNlcjpwdw==")
            .await;
        assert!(head.starts_with("HTTP/1.1 401 Unauthorized"), "got {head}");
        assert!(client.read_to_close().await.is_empty());
    });
}

// a user's sixth concurrent connection -> 101, then closed with no frame
#[test]
fn sixth_connection_of_a_user_is_closed_after_upgrade() {
    run(Options::default(), |gateway| async move {
        let mut open = Vec::new();
        for _ in 0..5 {
            open.push(gateway.connect(7).await);
        }
        let (head, mut sixth) = gateway.upgrade(Some(&token(7))).await;
        assert!(head.starts_with("HTTP/1.1 101"), "got {head}");
        assert!(sixth.read_to_close().await.is_empty());
        assert_eq!(gateway.connection_count(), 5);
        let other_user = gateway.connect(8).await;
        drop(other_user);
    });
}

// client drops its socket -> the session leaves the state
#[test]
fn closed_socket_removes_the_session() {
    run(Options::default(), |gateway| async move {
        let client = gateway.connect(7).await;
        assert_eq!(gateway.connection_count(), 1);
        drop(client);
        gateway
            .wait_until("session removed", |gateway| gateway.connection_count() == 0)
            .await;
    });
}

// upgrade request whose last header arrives 50 ms after the rest -> 101, as for one write
#[test]
fn upgrade_request_split_across_reads_is_accepted() {
    run(Options::default(), |gateway| async move {
        let (head, _client) = gateway.upgrade_in_two_writes(&token(7)).await;
        assert!(head.starts_with("HTTP/1.1 101"), "got {head}");
    });
}

// upgrade request with a 9 KiB header -> 431 and the socket is closed
#[test]
fn oversized_upgrade_request_is_answered_431() {
    run(Options::default(), |gateway| async move {
        let padding = format!("X-Padding: {}", "a".repeat(9 * 1024));
        let (head, _client) = gateway.upgrade_with_header(&padding).await;
        assert!(head.starts_with("HTTP/1.1 431"), "got {head}");
    });
}

// client that sends part of the request head and stalls -> the gateway closes the socket at the upgrade deadline
#[test]
fn stalled_upgrade_request_is_closed_at_the_deadline() {
    run(Options::default(), |gateway| async move {
        let mut client = TcpStream::connect(gateway.addr).await.unwrap();
        let (written, _) = client
            .write_all(b"GET /ws/private HTTP/1.1\r\n".to_vec())
            .await;
        written.unwrap();
        let started = std::time::Instant::now();

        let (read, _) = client.read(vec![0u8; 16]).await;

        assert_eq!(read.unwrap(), 0);
        assert!(started.elapsed() >= WS_UPGRADE_DEADLINE);
        assert!(started.elapsed() < WS_UPGRADE_DEADLINE + Duration::from_secs(2));
    });
}

// idle-reaped session (state removed as the main loop does) -> its socket is closed, an order sent on it is not forwarded
#[test]
fn reaped_session_socket_is_closed() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.state.borrow_mut().remove_connection(0);

        client.send_text(&order(&cid(1))).await;

        assert!(client.read_to_close().await.is_empty());
        gateway.risk.assert_silent();
    });
}

// idle reap at a cutoff -> sessions silent since before it are closed, a session active after it is served
#[test]
fn idle_reap_closes_only_the_silent_sessions() {
    run(Options::default(), |gateway| async move {
        let mut silent = gateway.connect(7).await;
        let mut active = gateway.connect(8).await;
        gateway.state.borrow_mut().touch_connection(1, u64::MAX);

        let reaped = gateway
            .state
            .borrow_mut()
            .reap_idle_connections(u64::MAX - 1);

        assert_eq!(reaped, 1);
        assert!(silent.read_to_close().await.is_empty());
        assert_eq!(active.settle().await, Vec::<String>::new());
        assert_eq!(gateway.connection_count(), 1);
    });
}

// reaped while the client has sent only part of a frame -> socket closed
#[test]
fn reaped_session_stalled_mid_frame_is_closed() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        client.send_partial_frame(&order(&cid(1))).await;
        monoio::time::sleep(std::time::Duration::from_millis(20)).await;

        gateway.state.borrow_mut().reap_idle_connections(u64::MAX);

        assert!(client.read_to_close().await.is_empty());
        gateway.risk.assert_silent();
    });
}

// housekeeping heartbeat -> each session gets one H with that timestamp, and the greeting at connect was the only other H
#[test]
fn broadcast_heartbeat_reaches_each_session_once() {
    run(Options::default(), |gateway| async move {
        let mut first = gateway.connect(7).await;
        let mut second = gateway.connect(8).await;
        first.assert_quiet().await;
        second.assert_quiet().await;

        gateway.state.borrow_mut().broadcast_heartbeat(1234);

        assert_eq!(first.recv().await, r#"{"H":[1234]}"#);
        assert_eq!(second.recv().await, r#"{"H":[1234]}"#);
        first.assert_quiet().await;
        second.assert_quiet().await;
    });
}

// client that never reads while updates keep arriving -> closed once its outbound queue reaches the cap
#[test]
fn client_that_never_reads_is_closed_at_the_outbound_cap() {
    let options = Options {
        max_outbound: 4,
        ..Options::default()
    };
    run(options, |gateway| async move {
        let mut client = gateway.connect(7).await;
        let update: std::sync::Arc<str> = "u".repeat(1 << 20).into();
        for _ in 0..2000 {
            if gateway.connection_count() == 0 {
                break;
            }
            gateway.state.borrow_mut().push_to_user(7, update.clone());
            monoio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert_eq!(gateway.connection_count(), 0);

        client.read_to_close().await;
    });
}
