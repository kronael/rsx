use crate::upgrade::compute_accept_key;
use crate::upgrade::header_value;
use crate::upgrade::is_ws_upgrade;
use crate::upgrade::read_http_request;
use crate::upgrade::ws_upgrade;
use crate::upgrade::WS_UPGRADE_DEADLINE;
use monoio::io::AsyncReadRent;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use std::future::Future;
use std::io;
use std::time::Duration;

const REQUEST: &str = "GET /ws HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
\r\n";

fn run<F, Fut>(body: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();
    runtime.block_on(async move { body().await });
}

async fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (client, server)
}

async fn send(client: &mut TcpStream, bytes: &[u8]) {
    let (written, _) = client.write_all(bytes.to_vec()).await;
    written.unwrap();
}

async fn read_some(client: &mut TcpStream) -> String {
    let (read, buf) = client.read(vec![0u8; 1024]).await;
    String::from_utf8_lossy(&buf[..read.unwrap()]).into_owned()
}

// RFC 6455 section 1.3 sample key -> the sample accept value
#[test]
fn accept_key_matches_the_rfc_sample() {
    assert_eq!(
        compute_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
}

// mixed-case header name with padded value -> trimmed value; absent header -> None
#[test]
fn header_value_is_case_insensitive_and_trimmed() {
    assert_eq!(
        header_value(
            "GET / HTTP/1.1\r\nSEC-websocket-KEY:  abc \r\n\r\n",
            "sec-websocket-key"
        ),
        Some("abc")
    );
    assert_eq!(header_value(REQUEST, "authorization"), None);
    assert!(is_ws_upgrade(REQUEST));
    assert!(!is_ws_upgrade("GET /health HTTP/1.1\r\n\r\n"));
}

// request head sent in two writes, plus a frame byte past the blank line -> whole head and the leftover
#[test]
fn split_request_is_read_whole_with_leftover() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        let (first, second) = REQUEST.as_bytes().split_at(20);
        send(&mut client, first).await;
        let writer = monoio::spawn(async move {
            monoio::time::sleep(Duration::from_millis(50)).await;
            send(&mut client, &[second, &[0x81]].concat()).await;
            client
        });

        let (request, leftover) = read_http_request(&mut server).await.unwrap();

        assert_eq!(request, REQUEST);
        assert_eq!(leftover, [0x81]);
        writer.await;
    });
}

// split request through ws_upgrade -> 101 with the accept key
#[test]
fn split_request_is_upgraded() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        let (first, second) = REQUEST.as_bytes().split_at(30);
        send(&mut client, first).await;
        let writer = monoio::spawn(async move {
            monoio::time::sleep(Duration::from_millis(50)).await;
            send(&mut client, second).await;
            client
        });

        let leftover = ws_upgrade(&mut server).await.unwrap();
        let mut client = writer.await;

        assert!(leftover.is_empty());
        let reply = read_some(&mut client).await;
        assert!(reply.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        assert!(reply.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
    });
}

// request without Sec-WebSocket-Key -> InvalidData
#[test]
fn request_without_key_is_refused() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        send(&mut client, b"GET /ws HTTP/1.1\r\n\r\n").await;
        let err = ws_upgrade(&mut server).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    });
}

// request head of 9 KiB -> 431 written, InvalidData returned
#[test]
fn oversized_head_is_answered_431() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        let head = format!(
            "GET /ws HTTP/1.1\r\nX-Padding: {}\r\n\r\n",
            "a".repeat(9 * 1024)
        );
        send(&mut client, head.as_bytes()).await;

        let err = read_http_request(&mut server).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(read_some(&mut client).await.starts_with("HTTP/1.1 431"));
    });
}

// head with no blank line past 8 KiB -> 431
#[test]
fn unterminated_head_over_the_cap_is_answered_431() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        send(&mut client, "a".repeat(9 * 1024).as_bytes()).await;

        let err = read_http_request(&mut server).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(read_some(&mut client).await.starts_with("HTTP/1.1 431"));
    });
}

// client that sends nothing after connecting -> TimedOut at the deadline and the socket closes
#[test]
fn silent_client_times_out_at_the_upgrade_deadline() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        let started = std::time::Instant::now();

        let err = ws_upgrade(&mut server).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= WS_UPGRADE_DEADLINE);
        assert!(started.elapsed() < WS_UPGRADE_DEADLINE + Duration::from_secs(2));
        drop(server);
        let (read, _) = client.read(vec![0u8; 16]).await;
        assert_eq!(read.unwrap(), 0);
    });
}
