use crate::{generator::OrderGenerator, metrics::Metrics, StressClient};
use anyhow::Result;
use std::time::Duration;
use tokio::time::interval;

pub struct WorkerConfig {
    pub gateway_url: String,
    pub user_id: u32,
    pub rate_per_sec: f64,
    pub duration_secs: u64,
    pub generator: OrderGenerator,
}

pub async fn worker_task(config: WorkerConfig) -> Result<Metrics> {
    let mut metrics = Metrics::new(None)?;
    let mut client = StressClient::new(config.gateway_url, config.user_id);
    let mut generator = config.generator;

    client.connect().await?;
    tracing::info!(
        user_id = config.user_id,
        rate = config.rate_per_sec,
        duration = config.duration_secs,
        "worker started"
    );

    let interval_ms = if config.rate_per_sec > 0.0 {
        (1000.0 / config.rate_per_sec) as u64
    } else {
        1000
    };

    let mut tick = interval(Duration::from_millis(interval_ms));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(config.duration_secs);

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        tick.tick().await;

        let order = generator.next_order();
        metrics.record_submitted();

        match client.submit_order(order.clone()).await {
            Ok(latency) => {
                metrics.record_accepted();
                metrics.record_latency(latency, "accepted", &order.client_order_id)?;
            }
            Err(e) => {
                metrics.record_error();
                tracing::warn!(error = %e, "order submission failed");
            }
        }
    }

    client.close().await?;
    tracing::info!(user_id = config.user_id, "worker finished");
    Ok(metrics)
}
