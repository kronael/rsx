use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::Instant;

/// Binds a listener on a free local port; returns it with its `host:port`.
pub async fn bind() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind health");
    let address = listener.local_addr().expect("health address").to_string();
    (listener, address)
}

/// Answers the health endpoints of a daemon: `/ready` and `/health` give 503
/// until `ready_after` has passed since the call and 200 after, `/metrics`
/// gives the JSON state `state`, anything else 404.
pub async fn serve(listener: TcpListener, ready_after: Duration, state: &'static str) {
    let started = Instant::now();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut buffer = [0u8; 512];
        let Ok(read) = stream.read(&mut buffer).await else {
            continue;
        };
        let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
        let path = request.split_whitespace().nth(1).unwrap_or("");
        let is_ready = started.elapsed() >= ready_after;
        let (status, body) = match path {
            "/ready" | "/health" if is_ready => ("200 OK", r#"{"status":"ok"}"#.to_string()),
            "/ready" | "/health" => ("503 Service Unavailable", r#"{"status":"no"}"#.to_string()),
            "/metrics" => ("200 OK", format!(r#"{{"state":"{state}"}}"#)),
            _ => ("404 Not Found", String::new()),
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        if let Err(error) = stream.write_all(response.as_bytes()).await {
            eprintln!("health fixture write failed: {error}");
        }
    }
}

/// A free local port, found by binding port 0 and releasing it.
pub fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
    listener.local_addr().expect("free port address").port()
}

/// Like a daemon booting: nothing listens for `delay`, then `serve` runs on
/// the local `port`.
pub async fn serve_later(port: u16, delay: Duration, ready_after: Duration, state: &'static str) {
    tokio::time::sleep(delay).await;
    let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await else {
        eprintln!("health fixture could not bind port {port}");
        return;
    };
    serve(listener, ready_after, state).await;
}
