//! The public WebSocket upgrade over real sockets: `handle_connection` on a
//! monoio loopback listener reads the whole request head the way the gateway
//! does, answers an oversized head `431`, and serves a subscribe that follows
//! the upgrade.

use monoio::io::AsyncReadRent;
use monoio::io::AsyncReadRentExt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use rsx_marketdata::handler::handle_connection;
use rsx_marketdata::state::MarketDataState;
use rsx_types::SymbolConfig;
use std::cell::RefCell;
use std::future::Future;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

const MASK: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
const REQUEST: &str = "GET / HTTP/1.1\r\n\
Host: localhost\r\n\
Upgrade: websocket\r\n\
Connection: Upgrade\r\n\
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\
\r\n";

fn run<F, Fut>(body: F)
where
    F: FnOnce(SocketAddr) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut runtime = monoio::RuntimeBuilder::<monoio::FusionDriver>::new()
        .enable_timer()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let config = SymbolConfig {
            symbol_id: 0,
            price_decimals: 2,
            qty_decimals: 2,
            tick_size: 1,
            lot_size: 1,
        };
        let mut state = MarketDataState::new(10, config, 100, 50000);
        state.ensure_book(1, 50000);
        let state = Rc::new(RefCell::new(state));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        monoio::spawn(async move {
            loop {
                let (stream, peer) = listener.accept().await.unwrap();
                monoio::spawn(handle_connection(stream, peer, state.clone(), 1024, 10));
            }
        });
        body(addr).await;
    });
}

async fn send(client: &mut TcpStream, bytes: Vec<u8>) {
    let (written, _) = client.write_all(bytes).await;
    written.unwrap();
}

fn masked_text(payload: &str) -> Vec<u8> {
    let mut frame = vec![0x81, 0x80 | payload.len() as u8];
    frame.extend_from_slice(&MASK);
    for (i, byte) in payload.bytes().enumerate() {
        frame.push(byte ^ MASK[i % 4]);
    }
    frame
}

async fn read_head(client: &mut TcpStream) -> String {
    let (read, buf) = client.read(vec![0u8; 1024]).await;
    String::from_utf8_lossy(&buf[..read.unwrap()]).into_owned()
}

/// First two bytes of the next server frame: opcode byte and length byte.
async fn read_frame_head(client: &mut TcpStream) -> [u8; 2] {
    let (read, head) = client.read_exact(vec![0u8; 2]).await;
    read.unwrap();
    [head[0], head[1]]
}

// upgrade request sent in two writes, then a depth subscribe -> 101, then a binary snapshot
#[test]
fn split_upgrade_request_is_accepted_and_subscribes() {
    run(|addr| async move {
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (first, second) = REQUEST.split_at(40);
        send(&mut client, first.as_bytes().to_vec()).await;
        monoio::time::sleep(Duration::from_millis(50)).await;
        send(&mut client, second.as_bytes().to_vec()).await;

        let head = read_head(&mut client).await;
        assert!(head.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        assert!(head.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));

        send(&mut client, masked_text(r#"{"S":[1,2]}"#)).await;
        let [opcode, _] = read_frame_head(&mut client).await;
        assert_eq!(opcode, 0x82);
    });
}

// upgrade request with a 9 KiB header -> 431 and the socket is closed
#[test]
fn oversized_upgrade_request_is_answered_431() {
    run(|addr| async move {
        let mut client = TcpStream::connect(addr).await.unwrap();
        let padding = format!("X-Padding: {}\r\n", "a".repeat(9 * 1024));
        let request = REQUEST.replace("\r\n\r\n", &format!("\r\n{padding}\r\n"));
        let (first, second) = request.split_at(8000);
        send(&mut client, first.as_bytes().to_vec()).await;
        monoio::time::sleep(Duration::from_millis(50)).await;
        send(&mut client, second.as_bytes().to_vec()).await;

        let head = read_head(&mut client).await;

        assert!(head.starts_with("HTTP/1.1 431"), "got {head}");
        let (read, _) = client.read(vec![0u8; 16]).await;
        assert_eq!(read.unwrap(), 0);
    });
}

// subscribe sent in the same write as the upgrade request -> answered with a snapshot
#[test]
fn pipelined_subscribe_is_answered() {
    run(|addr| async move {
        let mut client = TcpStream::connect(addr).await.unwrap();
        let mut bytes = REQUEST.as_bytes().to_vec();
        bytes.extend(masked_text(r#"{"S":[1,2]}"#));
        send(&mut client, bytes).await;
        read_head(&mut client).await;

        let answer =
            monoio::time::timeout(Duration::from_millis(500), read_frame_head(&mut client)).await;

        assert_eq!(answer.unwrap()[0], 0x82);
    });
}
