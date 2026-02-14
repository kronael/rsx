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

fn make_jwt(user_id: u32, exp: u64, secret: &str) -> String {
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        aud: Some("rsx-gateway".to_string()),
        iss: Some("rsx".to_string()),
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
    let secret = "test-secret";
    let user_id = 12345u32;
    let exp = now_secs() + 3600;
    let token = make_jwt(user_id, exp, secret);

    let mut runtime = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind(
            "127.0.0.1:0",
        )
        .unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) =
                listener.accept().await.unwrap();
            let result =
                ws_handshake(&mut stream, secret).await;
            assert!(result.is_ok());
            let (_key, uid) = result.unwrap();
            assert_eq!(uid, user_id);
        });

        monoio::time::sleep(
            std::time::Duration::from_millis(10),
        )
        .await;

        let mut client = TcpStream::connect(local_addr)
            .await
            .unwrap();

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
        let (res, _) =
            client.write_all(request.into_bytes()).await;
        res.unwrap();

        monoio::time::sleep(
            std::time::Duration::from_millis(100),
        )
        .await;
    });
}

#[test]
fn test_ws_handshake_with_expired_jwt() {
    let secret = "test-secret";
    let user_id = 12345u32;
    let exp = now_secs().saturating_sub(3600);
    let token = make_jwt(user_id, exp, secret);

    let mut runtime = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind(
            "127.0.0.1:0",
        )
        .unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) =
                listener.accept().await.unwrap();
            let result =
                ws_handshake(&mut stream, secret).await;
            assert!(result.is_err());
        });

        monoio::time::sleep(
            std::time::Duration::from_millis(10),
        )
        .await;

        let mut client = TcpStream::connect(local_addr)
            .await
            .unwrap();

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
        let (res, _) =
            client.write_all(request.into_bytes()).await;
        res.unwrap();

        monoio::time::sleep(
            std::time::Duration::from_millis(100),
        )
        .await;
    });
}

#[test]
fn test_ws_handshake_missing_auth() {
    let secret = "test-secret";

    let mut runtime = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind(
            "127.0.0.1:0",
        )
        .unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) =
                listener.accept().await.unwrap();
            let result =
                ws_handshake(&mut stream, secret).await;
            assert!(result.is_err());
        });

        monoio::time::sleep(
            std::time::Duration::from_millis(10),
        )
        .await;

        let mut client = TcpStream::connect(local_addr)
            .await
            .unwrap();

        let request =
            "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n";

        use monoio::io::AsyncWriteRentExt;
        let (res, _) = client
            .write_all(request.as_bytes().to_vec())
            .await;
        res.unwrap();

        monoio::time::sleep(
            std::time::Duration::from_millis(100),
        )
        .await;
    });
}

#[test]
fn test_ws_handshake_x_user_id_fallback() {
    let secret = "test-secret";
    let user_id = 67890u32;

    let mut runtime = monoio::RuntimeBuilder::<
        monoio::FusionDriver,
    >::new()
    .enable_timer()
    .build()
    .unwrap();

    runtime.block_on(async move {
        let listener = monoio::net::TcpListener::bind(
            "127.0.0.1:0",
        )
        .unwrap();
        let local_addr = listener.local_addr().unwrap();

        monoio::spawn(async move {
            let (mut stream, _) =
                listener.accept().await.unwrap();
            let result =
                ws_handshake(&mut stream, secret).await;
            assert!(result.is_ok());
            let (_key, uid) = result.unwrap();
            assert_eq!(uid, user_id);
        });

        monoio::time::sleep(
            std::time::Duration::from_millis(10),
        )
        .await;

        let mut client = TcpStream::connect(local_addr)
            .await
            .unwrap();

        let request = format!(
            "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
X-User-Id: {}\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n",
            user_id
        );

        use monoio::io::AsyncWriteRentExt;
        let (res, _) =
            client.write_all(request.into_bytes()).await;
        res.unwrap();

        monoio::time::sleep(
            std::time::Duration::from_millis(100),
        )
        .await;
    });
}
