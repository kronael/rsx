use jsonwebtoken::encode;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use monoio::net::TcpStream;
use rsx_gateway::jwt::Claims;
use rsx_gateway::ws::ws_handshake;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
// tokio not needed in tests

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Mint a JWT matching what rsx-auth ships (aud=rsx-gateway,
/// iss=rsx-auth, unique jti). The gateway rejects tokens
/// without a `jti` — see ws.rs::extract_user_and_record_jti
/// and CTO-REPORT.md R3.
fn make_jwt(user_id: u32, exp: u64, secret: &str) -> String {
    let jti = format!(
        "ws-e2e-{}-{}-{}",
        user_id,
        exp,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    );
    make_jwt_with_jti(user_id, exp, &jti, secret)
}

/// Mint a token deliberately missing the `jti` claim — used
/// to assert that the gateway rejects such tokens (F2.2).
fn make_jwt_no_jti(user_id: u32, exp: u64, secret: &str) -> String {
    let claims = Claims {
        sub: format!("github:{user_id}"),
        user_id: Some(user_id),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
        nbf: None,
        jti: None,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

#[test]
fn test_ws_handshake_with_valid_jwt() {
    let secret = "test-secret-padded-to-32-bytes-minlen!";
    let user_id = 12345u32;
    let exp = now_secs() + 3600;
    let token = make_jwt(user_id, exp, secret);

    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let result = ws_handshake(&mut stream, secret).await;
            assert!(result.is_ok());
            let (_key, uid, _leftover) = result.unwrap();
            assert_eq!(uid, user_id);
        });

        monoio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut client = TcpStream::connect(local_addr).await.unwrap();

        let request = format!(
            "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Authorization: Bearer {}\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n",
            token
        );

        use monoio::io::AsyncWriteRentExt;
        let (res, _) = client.write_all(request.into_bytes()).await;
        res.unwrap();

        monoio::time::sleep(std::time::Duration::from_millis(100)).await;
    });
}

#[test]
fn test_ws_handshake_with_expired_jwt() {
    let secret = "test-secret-padded-to-32-bytes-minlen!";
    let user_id = 12345u32;
    let exp = now_secs().saturating_sub(3600);
    let token = make_jwt(user_id, exp, secret);

    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let result = ws_handshake(&mut stream, secret).await;
            assert!(result.is_err());
        });

        monoio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut client = TcpStream::connect(local_addr).await.unwrap();

        let request = format!(
            "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Authorization: Bearer {}\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n",
            token
        );

        use monoio::io::AsyncWriteRentExt;
        let (res, _) = client.write_all(request.into_bytes()).await;
        res.unwrap();

        monoio::time::sleep(std::time::Duration::from_millis(100)).await;
    });
}

#[test]
fn test_ws_handshake_missing_auth() {
    let secret = "test-secret-padded-to-32-bytes-minlen!";

    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let result = ws_handshake(&mut stream, secret).await;
            assert!(result.is_err());
        });

        monoio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut client = TcpStream::connect(local_addr).await.unwrap();

        let request = "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n";

        use monoio::io::AsyncWriteRentExt;
        let (res, _) = client.write_all(request.as_bytes().to_vec()).await;
        res.unwrap();

        monoio::time::sleep(std::time::Duration::from_millis(100)).await;
    });
}

/// Mint a JWT with an explicit `jti` claim — used by the
/// replay test below.
fn make_jwt_with_jti(user_id: u32, exp: u64, jti: &str, secret: &str) -> String {
    let claims = Claims {
        sub: format!("github:{user_id}"),
        user_id: Some(user_id),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx-auth".to_string()),
        nbf: None,
        jti: Some(jti.to_string()),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap()
}

/// Replay protection: two handshakes with the SAME token
/// must yield (ok, err). The second one trips the process-
/// wide JtiTracker installed in rsx-gateway::ws.
#[test]
fn test_ws_handshake_rejects_jti_replay() {
    let secret = "test-secret-padded-to-32-bytes-minlen!";
    let user_id = 77777u32;
    let exp = now_secs() + 3600;
    // Unique jti per run so the static tracker doesn't
    // collide across test invocations.
    let jti = format!("replay-test-{}", now_secs());
    let token = make_jwt_with_jti(user_id, exp, &jti, secret);

    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local_addr = listener.local_addr().unwrap();

        // Serve two consecutive handshakes; first ok, second err.
        monoio::spawn(async move {
            // First.
            let (mut s1, _) = listener.accept().await.unwrap();
            let r1 = ws_handshake(&mut s1, secret).await;
            assert!(r1.is_ok(), "first handshake should succeed");
            // Second uses the same jti.
            let (mut s2, _) = listener.accept().await.unwrap();
            let r2 = ws_handshake(&mut s2, secret).await;
            assert!(r2.is_err(), "second handshake should reject jti replay");
        });

        monoio::time::sleep(std::time::Duration::from_millis(10)).await;

        let request = format!(
            "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Authorization: Bearer {}\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n",
            token
        );

        use monoio::io::AsyncWriteRentExt;

        // First connection.
        let mut c1 = TcpStream::connect(local_addr).await.unwrap();
        let (r1, _) = c1.write_all(request.clone().into_bytes()).await;
        r1.unwrap();
        monoio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(c1);

        // Second connection: same token, same jti.
        let mut c2 = TcpStream::connect(local_addr).await.unwrap();
        let (r2, _) = c2.write_all(request.into_bytes()).await;
        r2.unwrap();
        monoio::time::sleep(std::time::Duration::from_millis(100)).await;
    });
}

/// F2.2: tokens without a `jti` claim must be rejected. The
/// previous JtiTracker contract let them through, which made
/// the replay defence null-defeated. See CTO-REPORT.md R3 and
/// SYNTHESIS.md F2.2.
#[test]
fn test_ws_handshake_rejects_missing_jti() {
    let secret = "test-secret-padded-to-32-bytes-minlen!";
    let user_id = 88888u32;
    let exp = now_secs() + 3600;
    let token = make_jwt_no_jti(user_id, exp, secret);

    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let result = ws_handshake(&mut stream, secret).await;
            assert!(result.is_err(), "token without jti must be rejected");
            let err = result.unwrap_err();
            assert!(
                err.to_string().contains("missing jti"),
                "expected 'missing jti', got: {err}"
            );
        });

        monoio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut client = TcpStream::connect(local_addr).await.unwrap();

        let request = format!(
            "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Authorization: Bearer {}\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n",
            token
        );

        use monoio::io::AsyncWriteRentExt;
        let (res, _) = client.write_all(request.into_bytes()).await;
        res.unwrap();

        // Read the 401 response to demonstrate the negative
        // case ("token WITHOUT jti gets 401").
        use monoio::io::AsyncReadRent;
        let buf = vec![0u8; 128];
        let (res, buf) = client.read(buf).await;
        let n = res.unwrap();
        let resp = String::from_utf8_lossy(&buf[..n]);
        assert!(
            resp.starts_with("HTTP/1.1 401"),
            "expected 401, got: {resp}"
        );
    });
}
