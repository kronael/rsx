use monoio::io::AsyncReadRentExt;
use monoio::io::AsyncWriteRentExt;
use monoio::net::TcpStream;
use std::io;

const MAX_FRAME_LEN: usize = 4096;

/// True when `buf` already decides the next `ws_read_frame_buf`: it holds a
/// whole frame, or a header the reader refuses. A handler skips its readiness
/// wait while this holds, so a frame sent in the same segment as the upgrade
/// request is answered at once.
pub fn is_frame_buffered(buf: &[u8]) -> bool {
    let (Some(&first), Some(&second)) = (buf.first(), buf.get(1)) else {
        return false;
    };
    if first & 0x80 == 0 || second & 0x80 == 0 {
        return true;
    }
    let (length_bytes, payload_len) = match second & 0x7F {
        126 => (2, buf.get(2..4).map(big_endian_len)),
        127 => (8, buf.get(2..10).map(big_endian_len)),
        len => (0, Some(usize::from(len))),
    };
    match payload_len {
        None => false,
        Some(len) if len > MAX_FRAME_LEN => true,
        Some(len) => buf.len() >= 2 + length_bytes + 4 + len,
    }
}

/// Read one client frame, consuming from `buf` before the stream. Returns
/// `(opcode, unmasked payload)`. Dropping the future mid-frame loses the
/// bytes already taken, so the caller awaits readability first and then
/// runs this to the end. Fragmented, unmasked and over-long frames are
/// `InvalidData` errors.
pub async fn ws_read_frame_buf(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
) -> io::Result<(u8, Vec<u8>)> {
    let head = take_bytes(stream, buf, 2).await?;
    if head[0] & 0x80 == 0 {
        return Err(invalid_frame("fragmented frame (FIN=0) not supported"));
    }
    if head[1] & 0x80 == 0 {
        return Err(invalid_frame("unmasked client frame rejected"));
    }
    let opcode = head[0] & 0x0F;
    let len1 = usize::from(head[1] & 0x7F);

    let payload_len = match len1 {
        126 => big_endian_len(&take_bytes(stream, buf, 2).await?),
        127 => big_endian_len(&take_bytes(stream, buf, 8).await?),
        _ => len1,
    };
    if payload_len > MAX_FRAME_LEN {
        return Err(invalid_frame("frame exceeds 4096 bytes"));
    }

    let mask = take_bytes(stream, buf, 4).await?;
    let mut payload = take_bytes(stream, buf, payload_len).await?;
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[i % 4];
    }

    Ok((opcode, payload))
}

async fn take_bytes(stream: &mut TcpStream, buf: &mut Vec<u8>, len: usize) -> io::Result<Vec<u8>> {
    let take = len.min(buf.len());
    let mut out: Vec<u8> = buf.drain(..take).collect();
    if out.len() < len {
        let (res, rest) = stream.read_exact(vec![0u8; len - out.len()]).await;
        res?;
        out.extend_from_slice(&rest);
    }
    Ok(out)
}

fn big_endian_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .fold(0, |len, byte| (len << 8) | usize::from(*byte))
}

fn invalid_frame(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

/// Write a text frame.
pub async fn ws_write_text(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    ws_write_frame(stream, 0x1, data).await
}

/// Write a binary frame.
pub async fn ws_write_binary(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    ws_write_frame(stream, 0x2, data).await
}

/// Write one unmasked FIN frame with the given opcode: data, close, ping or
/// pong.
pub async fn ws_write_frame(stream: &mut TcpStream, opcode: u8, data: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(10 + data.len());
    frame.push(0x80 | (opcode & 0x0F));
    if data.len() <= 125 {
        frame.push(data.len() as u8);
    } else if data.len() <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(data.len() as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(data.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(data);

    let (res, _) = stream.write_all(frame).await;
    res?;
    Ok(())
}

#[cfg(test)]
#[path = "frame_test.rs"]
mod frame_test;
