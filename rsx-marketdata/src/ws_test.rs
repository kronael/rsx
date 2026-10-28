use crate::ws::ws_handshake;
use crate::ws::ws_read_frame;
use monoio::io::AsyncReadRent;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use rsx_types::time_utils::WS_UPGRADE_DEADLINE;
use std::future::Future;
use std::io;
use std::time::Duration;

const MASK: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

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

/// Bytes the client sends, read back by `ws_read_frame` on the accepted side.
async fn read_sent_frame(sent: Vec<u8>) -> io::Result<(u8, Vec<u8>)> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut server, _) = listener.accept().await.unwrap();
    let (written, _) = client.write_all(sent).await;
    written.unwrap();
    ws_read_frame(&mut server).await
}

fn masked_frame(first: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![first];
    if payload.len() <= 125 {
        frame.push(0x80 | payload.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&MASK);
    for (i, byte) in payload.iter().enumerate() {
        frame.push(byte ^ MASK[i % 4]);
    }
    frame
}

// masked FIN text frame -> opcode and unmasked payload
#[test]
fn masked_frame_is_unmasked() {
    run(|| async {
        let (opcode, payload) = read_sent_frame(masked_frame(0x81, b"hello")).await.unwrap();
        assert_eq!(opcode, 1);
        assert_eq!(payload, b"hello");
    });
}

// masked frame of exactly 4096 bytes (16-bit length) -> accepted
#[test]
fn frame_at_the_cap_is_accepted() {
    run(|| async {
        let (_, payload) = read_sent_frame(masked_frame(0x81, &[b'a'; 4096]))
            .await
            .unwrap();
        assert_eq!(payload.len(), 4096);
    });
}

// frame without the mask bit -> InvalidData
#[test]
fn unmasked_frame_is_refused() {
    run(|| async {
        let err = read_sent_frame(vec![0x81, 0x02, b'h', b'i'])
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    });
}

// frame with FIN=0 -> InvalidData
#[test]
fn fragmented_frame_is_refused() {
    run(|| async {
        let err = read_sent_frame(masked_frame(0x01, b"hello"))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    });
}

// masked frame of 5 KiB -> InvalidData
#[test]
fn frame_over_the_cap_is_refused() {
    run(|| async {
        let err = read_sent_frame(masked_frame(0x81, &[b'a'; 5 * 1024]))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    });
}

// frame with a 64-bit length field -> InvalidData before any payload is read
#[test]
fn eight_byte_length_is_refused() {
    run(|| async {
        let mut frame = vec![0x81, 0x80 | 127];
        frame.extend_from_slice(&u64::MAX.to_be_bytes());
        let err = read_sent_frame(frame).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    });
}

// client that sends nothing after connecting -> handshake fails TimedOut at the deadline and the socket closes
#[test]
fn silent_client_times_out_at_the_upgrade_deadline() {
    run(|| async {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        let started = std::time::Instant::now();

        let err = ws_handshake(&mut server).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= WS_UPGRADE_DEADLINE);
        assert!(started.elapsed() < WS_UPGRADE_DEADLINE + Duration::from_secs(2));
        drop(server);
        let (read, _) = client.read(vec![0u8; 16]).await;
        assert_eq!(read.unwrap(), 0);
    });
}
