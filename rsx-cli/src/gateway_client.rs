use crate::output::Failure;
use futures_util::SinkExt;
use futures_util::StreamExt;
use rsx_messages::parse;
use rsx_messages::serialize;
use rsx_messages::WsFrame;
use serde_json::Value;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio::time::timeout_at;
use tokio::time::Instant;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// One frame from the gateway: the parsed frame and the JSON as received.
pub struct Received {
    pub frame: WsFrame,
    pub raw: Value,
}

/// An authenticated order-channel WebSocket. Reading answers every gateway
/// heartbeat with one of the client's own, so a connection that waits longer
/// than the gateway's idle timeout stays open.
pub struct Connection {
    socket: Socket,
    url: String,
}

impl Connection {
    /// Opens the order channel at `url` with `token` as the bearer token.
    /// A refused handshake (HTTP 401 for a bad token) is a `Runtime` failure.
    pub async fn open(url: &str, token: &str) -> Result<Self, Failure> {
        let mut request = url
            .into_client_request()
            .map_err(|error| Failure::misuse(format!("bad gateway url {url}: {error}")))?;
        let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|error| Failure::misuse(format!("bad token: {error}")))?;
        request.headers_mut().insert("Authorization", bearer);
        match timeout(CONNECT_TIMEOUT, connect_async(request)).await {
            Ok(Ok((socket, _))) => Ok(Self {
                socket,
                url: url.to_string(),
            }),
            Ok(Err(WsError::Http(response))) => Err(Failure::runtime(format!(
                "{url} refused the connection: HTTP {} (the token must be signed with the \
                 gateway's RSX_GW_JWT_SECRET)",
                response.status()
            ))),
            Ok(Err(error)) => Err(Failure::runtime(format!(
                "failed to connect to {url}: {error}"
            ))),
            Err(_) => Err(Failure::runtime(format!(
                "timed out connecting to {url} after {} s",
                CONNECT_TIMEOUT.as_secs()
            ))),
        }
    }

    /// Sends one client frame.
    pub async fn send(&mut self, frame: &WsFrame) -> Result<(), Failure> {
        self.send_text(serialize(frame)).await
    }

    /// The next gateway frame that is not a heartbeat, `None` when `deadline`
    /// passes first. A closed connection or an unparseable frame is a
    /// `Runtime` failure.
    pub async fn next_frame(&mut self, deadline: Instant) -> Result<Option<Received>, Failure> {
        let Some((frame, text)) = self.read_frame(deadline).await? else {
            return Ok(None);
        };
        let raw = serde_json::from_str(&text).map_err(|error| {
            Failure::runtime(format!("bad frame from {}: {error}: {text}", self.url))
        })?;
        Ok(Some(Received { frame, raw }))
    }

    /// Like `next_frame` without the raw JSON, for callers that count frames
    /// and do not keep them.
    pub async fn next_parsed(&mut self, deadline: Instant) -> Result<Option<WsFrame>, Failure> {
        Ok(self.read_frame(deadline).await?.map(|(frame, _)| frame))
    }

    async fn read_frame(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<(WsFrame, String)>, Failure> {
        loop {
            let message = match timeout_at(deadline, self.socket.next()).await {
                Ok(message) => message,
                Err(_) => return Ok(None),
            };
            match message {
                Some(Ok(Message::Text(text))) => {
                    let frame = parse(&text).map_err(|error| {
                        Failure::runtime(format!("bad frame from {}: {error}: {text}", self.url))
                    })?;
                    if matches!(frame, WsFrame::Heartbeat { .. }) {
                        self.echo_heartbeat().await?;
                        continue;
                    }
                    return Ok(Some((frame, text)));
                }
                Some(Ok(Message::Close(_))) | None => {
                    return Err(Failure::runtime(format!(
                        "connection to {} closed before the order finished",
                        self.url
                    )))
                }
                Some(Ok(_)) => continue,
                Some(Err(error)) => {
                    return Err(Failure::runtime(format!(
                        "read from {} failed: {error}",
                        self.url
                    )))
                }
            }
        }
    }

    async fn echo_heartbeat(&mut self) -> Result<(), Failure> {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        self.send(&WsFrame::Heartbeat { timestamp_ms }).await
    }

    async fn send_text(&mut self, text: String) -> Result<(), Failure> {
        self.socket
            .send(Message::Text(text))
            .await
            .map_err(|error| Failure::runtime(format!("failed to send to {}: {error}", self.url)))
    }
}
