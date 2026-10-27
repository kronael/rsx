use futures_util::SinkExt;
use futures_util::StreamExt;
use std::net::TcpListener as StdListener;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// What the fake marketdata does once it sent its frames.
#[derive(Clone, Copy)]
pub enum After {
    Close,
    Hold,
}

pub struct MdServer {
    pub url: String,
    /// The text messages the client sent, in order.
    pub received: Arc<Mutex<Vec<String>>>,
}

/// A url nothing listens on.
pub fn closed_url() -> String {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    format!("ws://127.0.0.1:{port}")
}

/// Accepts one WebSocket client, waits for `subscriptions` subscribe
/// messages, sends `frames` as BINARY messages and then does `after`.
pub fn serve(subscriptions: usize, frames: Vec<Vec<u8>>, after: After) -> MdServer {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let received = Arc::new(Mutex::new(Vec::new()));
    let log = received.clone();
    thread::spawn(move || {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(serve_one(listener, subscriptions, frames, after, log));
    });
    MdServer { url, received }
}

async fn serve_one(
    listener: StdListener,
    subscriptions: usize,
    frames: Vec<Vec<u8>>,
    after: After,
    log: Arc<Mutex<Vec<String>>>,
) {
    let listener = TcpListener::from_std(listener).unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = accept_async(stream).await.unwrap();
    let mut subscribed = 0;
    while subscribed < subscriptions {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                if text.starts_with("{\"S\"") {
                    subscribed += 1;
                }
                log.lock().unwrap().push(text);
            }
            Some(Ok(_)) => {}
            _ => return,
        }
    }
    for frame in frames {
        socket.send(Message::Binary(frame)).await.unwrap();
    }
    match after {
        After::Close => {
            let _ = socket.close(None).await;
        }
        After::Hold => while let Some(Ok(_)) = socket.next().await {},
    }
}
