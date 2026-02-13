use crate::types::NewOrder;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};

pub struct StressClient {
    pub url: String,
    pub user_id: u32,
    ws: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
}

impl StressClient {
    pub fn new(url: String, user_id: u32) -> Self {
        Self {
            url,
            user_id,
            ws: None,
        }
    }

    pub async fn connect(&mut self) -> Result<()> {
        let (ws_stream, _) = connect_async(&self.url)
            .await
            .context("failed to connect to gateway")?;
        self.ws = Some(ws_stream);
        tracing::info!(url = %self.url, user_id = self.user_id, "connected");
        Ok(())
    }

    pub async fn submit_order(&mut self, order: NewOrder) -> Result<Duration> {
        let ws = self.ws.as_mut().context("not connected")?;

        let msg = json!({
            "N": [
                order.symbol_id,
                order.side,
                order.price,
                order.qty,
                order.client_order_id,
                order.tif,
                order.reduce_only,
                order.post_only
            ]
        });

        let start = Instant::now();
        ws.send(Message::Text(msg.to_string()))
            .await
            .context("failed to send order")?;

        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let _resp: Value = serde_json::from_str(&text)?;
                    return Ok(start.elapsed());
                }
                Some(Ok(Message::Close(_))) => {
                    anyhow::bail!("connection closed");
                }
                Some(Err(e)) => {
                    anyhow::bail!("websocket error: {}", e);
                }
                None => {
                    anyhow::bail!("connection closed");
                }
                _ => {}
            }
        }
    }

    pub async fn close(&mut self) -> Result<()> {
        if let Some(mut ws) = self.ws.take() {
            ws.close(None).await.context("failed to close ws")?;
        }
        Ok(())
    }
}
