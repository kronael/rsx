//! The connection handler's session lifecycle: a valid token opens a session
//! that greets with a heartbeat, any other token or none gets a 401 and a
//! closed socket, a user's sixth connection is closed after the 101, and a
//! closed socket ends the session.

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
#[ignore = "BUGS.md GW-UPGRADE-SINGLE-READ"]
fn known_defect_upgrade_request_split_across_reads_is_accepted() {
    run(Options::default(), |gateway| async move {
        let (head, _client) = gateway.upgrade_in_two_writes(&token(7)).await;
        assert!(head.starts_with("HTTP/1.1 101"), "got {head}");
    });
}

// idle-reaped session (state removed as the main loop does) -> its socket is closed, an order sent on it is not forwarded
#[test]
#[ignore = "BUGS.md GW-REAP-LEAKS-HANDLER"]
fn known_defect_reaped_session_socket_is_closed() {
    run(Options::default(), |gateway| async move {
        let mut client = gateway.connect(7).await;
        gateway.state.borrow_mut().remove_connection(0);

        client.send_text(&order(&cid(1))).await;

        assert!(client.read_to_close().await.is_empty());
        gateway.risk.assert_silent();
    });
}
