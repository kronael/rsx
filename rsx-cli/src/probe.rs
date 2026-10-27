use serde_json::Value;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

#[cfg(test)]
#[path = "probe_test.rs"]
mod probe_test;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// How a daemon says it is up: a primary answers 200 on `/ready` once it
/// serves traffic; a warm replica answers 200 on `/health` while it follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Ready,
    Live,
}

impl Probe {
    pub fn path(self) -> &'static str {
        match self {
            Probe::Ready => "/ready",
            Probe::Live => "/health",
        }
    }
}

/// The HTTP status of `GET path` at `address`, `None` when nothing answers
/// within the request timeout.
pub async fn status(address: &str, path: &str) -> Option<u16> {
    let (head, _) = get(address, path).await?;
    head.split_whitespace().nth(1)?.parse().ok()
}

/// The `state` label a daemon reports on `/metrics` (`warm_catchup`, `live`),
/// `None` when it does not answer or has none.
pub async fn state(address: &str) -> Option<String> {
    let (_, body) = get(address, "/metrics").await?;
    let metrics: Value = serde_json::from_str(&body).ok()?;
    metrics.get("state")?.as_str().map(str::to_string)
}

/// The value of the counter `name` on `/metrics`, `None` when the daemon does
/// not answer or has no such counter.
pub async fn counter(address: &str, name: &str) -> Option<u64> {
    let (_, body) = get(address, "/metrics").await?;
    counter_in(&body, name)
}

fn counter_in(metrics: &str, name: &str) -> Option<u64> {
    let metrics: Value = serde_json::from_str(metrics).ok()?;
    let counters = metrics.get("counters")?.as_array()?;
    let counter = counters
        .iter()
        .find(|counter| counter.get("name").and_then(Value::as_str) == Some(name))?;
    counter.get("value")?.as_u64()
}

/// True when `address` answers `probe` with 200.
pub async fn is_up(address: &str, probe: Probe) -> bool {
    status(address, probe.path()).await == Some(200)
}

async fn get(address: &str, path: &str) -> Option<(String, String)> {
    let exchange = async {
        let mut stream = TcpStream::connect(address).await.ok()?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.ok()?;
        let response = String::from_utf8_lossy(&response).into_owned();
        let (head, body) = response.split_once("\r\n\r\n")?;
        Some((head.to_string(), body.to_string()))
    };
    timeout(REQUEST_TIMEOUT, exchange).await.ok()?
}
