use crate::frame::is_frame_buffered;
use crate::frame::ws_read_frame_buf;
use crate::frame::ws_write_binary;
use crate::frame::ws_write_frame;
use crate::frame::ws_write_text;
use monoio::io::AsyncReadRentExt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpListener;
use monoio::net::TcpStream;
use std::future::Future;
use std::io;

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

async fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    (client, server)
}

/// Bytes the client sends, read back by `ws_read_frame_buf` on the accepted side.
async fn read_sent_frame(sent: Vec<u8>) -> io::Result<(u8, Vec<u8>)> {
    let (mut client, mut server) = connected_pair().await;
    let (written, _) = client.write_all(sent).await;
    written.unwrap();
    ws_read_frame_buf(&mut server, &mut Vec::new()).await
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

// frame whose first bytes are already in the buffer -> buffer consumed before the stream
#[test]
fn buffered_bytes_are_read_before_the_stream() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        let frame = masked_frame(0x81, b"hello");
        let mut buf = frame[..3].to_vec();
        let (written, _) = client.write_all(frame[3..].to_vec()).await;
        written.unwrap();

        let (opcode, payload) = ws_read_frame_buf(&mut server, &mut buf).await.unwrap();

        assert_eq!(opcode, 1);
        assert_eq!(payload, b"hello");
        assert!(buf.is_empty());
    });
}

// text, binary and pong writers -> FIN frames with their opcodes, 16-bit length past 125 bytes
#[test]
fn writers_emit_fin_frames_with_their_opcode() {
    run(|| async {
        let (mut client, mut server) = connected_pair().await;
        ws_write_text(&mut server, b"hi").await.unwrap();
        ws_write_binary(&mut server, &[7u8; 200]).await.unwrap();
        ws_write_frame(&mut server, 0xA, b"p").await.unwrap();

        let (read, text) = client.read_exact(vec![0u8; 4]).await;
        read.unwrap();
        assert_eq!(text, [0x81, 2, b'h', b'i']);
        let (read, binary) = client.read_exact(vec![0u8; 4 + 200]).await;
        read.unwrap();
        assert_eq!(binary[..4], [0x82, 126, 0, 200]);
        let (read, pong) = client.read_exact(vec![0u8; 3]).await;
        read.unwrap();
        assert_eq!(pong, [0x8A, 1, b'p']);
    });
}

// buffer holding a whole frame -> buffered; one byte short, or only a header -> not
#[test]
fn whole_frame_is_buffered() {
    let frame = masked_frame(0x81, b"hello");
    assert!(is_frame_buffered(&frame));
    assert!(!is_frame_buffered(&frame[..frame.len() - 1]));
    assert!(!is_frame_buffered(&frame[..1]));
    assert!(!is_frame_buffered(&[]));
}

// 16-bit length frame -> buffered only once the payload is complete
#[test]
fn extended_length_frame_is_buffered_when_complete() {
    let frame = masked_frame(0x81, &[b'a'; 300]);
    assert!(is_frame_buffered(&frame));
    assert!(!is_frame_buffered(&frame[..frame.len() - 1]));
    assert!(!is_frame_buffered(&frame[..3]));
}

// header the reader refuses (unmasked, FIN=0, over the cap) -> buffered so the error is answered at once
#[test]
fn refused_header_is_buffered() {
    assert!(is_frame_buffered(&[0x81, 0x02]));
    assert!(is_frame_buffered(&[0x01, 0x82]));
    assert!(is_frame_buffered(&[0x81, 0x80 | 126, 0x20, 0x00]));
    let mut huge = vec![0x81, 0x80 | 127];
    huge.extend_from_slice(&u64::MAX.to_be_bytes());
    assert!(is_frame_buffered(&huge));
}
