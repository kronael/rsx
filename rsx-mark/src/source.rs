/// Price update from an exchange source connector.
pub struct SourcePrice {
    pub source_id: u8,
    pub price: i64,
    pub timestamp_ns: u64,
}

/// Exchange price feed connector.
///
/// Each implementation (Binance, Coinbase, etc.) connects
/// to an exchange WebSocket, parses price updates, and
/// pushes SourcePrice values to the aggregation loop via
/// an SPSC producer ring.
///
/// Reconnect is handled internally with exponential
/// backoff: 1s, 2s, 4s, 8s, capped at 30s. Reset on
/// successful message.
pub trait PriceSource {
    /// Start the connector. Pushes SourcePrice updates
    /// to the provided SPSC producer. Handles reconnects
    /// internally. Runs as an async task on a tokio
    /// runtime.
    fn start(
        &self,
        tx: rtrb::Producer<SourcePrice>,
    );
}
