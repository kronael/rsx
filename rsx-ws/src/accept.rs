use monoio::net::TcpListener;
use monoio::net::TcpStream;
use std::io;
use std::net::SocketAddr;
use tracing::info;
use tracing::warn;

/// Bind `addr` and call `handler` with every accepted connection and its peer
/// address, `TCP_NODELAY` set. Runs until `accept` fails; dropping the future
/// closes the listener.
pub async fn ws_accept_loop<F>(addr: &str, handler: F) -> io::Result<()>
where
    F: Fn(TcpStream, SocketAddr) + 'static,
{
    let listener = TcpListener::bind(addr)?;
    info!("ws listening on {}", addr);
    loop {
        let (stream, peer) = listener.accept().await?;
        if let Err(e) = stream.set_nodelay(true) {
            warn!("set_nodelay failed for {peer}: {e}");
        }
        info!("ws connection from {}", peer);
        handler(stream, peer);
    }
}

#[cfg(test)]
#[path = "accept_test.rs"]
mod accept_test;
