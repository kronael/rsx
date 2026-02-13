use clap::Parser;
use rsx_stress::{OrderGenerator, SymbolConfig, WorkerConfig};

#[derive(Parser)]
#[command(name = "rsx-stress")]
#[command(about = "WebSocket stress test client for RSX Gateway")]
struct Args {
    #[arg(long, default_value = "ws://localhost:8080")]
    gateway: String,

    #[arg(long, default_value = "1000")]
    rate: u64,

    #[arg(long, default_value = "60")]
    duration: u64,

    #[arg(long, default_value = "BTCUSD")]
    symbols: String,

    #[arg(long, default_value = "10")]
    users: u32,

    #[arg(long, default_value = "10")]
    connections: usize,

    #[arg(long, default_value = "stress-test.csv")]
    output: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    tracing::info!(
        "starting stress test: rate={}/s, duration={}s, connections={}",
        args.rate,
        args.duration,
        args.connections
    );

    let per_conn_rate = args.rate as f64 / args.connections as f64;

    let symbols = vec![
        SymbolConfig {
            symbol_id: 1,
            name: "BTCUSD".to_string(),
            mid_price: 50000_00,
            tick_size: 1_00,
            lot_size: 1_00,
            weight: 0.5,
        },
        SymbolConfig {
            symbol_id: 2,
            name: "ETHUSD".to_string(),
            mid_price: 3000_00,
            tick_size: 1_00,
            lot_size: 1_00,
            weight: 0.3,
        },
        SymbolConfig {
            symbol_id: 3,
            name: "SOLUSD".to_string(),
            mid_price: 100_00,
            tick_size: 1_00,
            lot_size: 1_00,
            weight: 0.2,
        },
    ];

    let users: Vec<u32> = (1001..=1000 + args.users).collect();

    let mut handles = vec![];
    for i in 0..args.connections {
        let gateway_url = args.gateway.clone();
        let user_id = users[i % users.len()];
        let generator = OrderGenerator::new(symbols.clone(), users.clone());

        let config = WorkerConfig {
            gateway_url,
            user_id,
            rate_per_sec: per_conn_rate,
            duration_secs: args.duration,
            generator,
        };

        let handle = tokio::spawn(async move { rsx_stress::worker_task(config).await });
        handles.push(handle);
    }

    tracing::info!("spawned {} workers", handles.len());

    let mut results = vec![];
    for handle in handles {
        match handle.await {
            Ok(Ok(metrics)) => results.push(metrics),
            Ok(Err(e)) => tracing::error!(error = %e, "worker failed"),
            Err(e) => tracing::error!(error = %e, "worker panicked"),
        }
    }

    let mut total_submitted = 0;
    let mut total_accepted = 0;
    let mut total_rejected = 0;
    let mut total_errors = 0;
    let mut all_p50 = vec![];
    let mut all_p95 = vec![];
    let mut all_p99 = vec![];

    for metrics in &results {
        let summary = metrics.summary();
        total_submitted += summary.total;
        total_accepted += summary.accepted;
        total_rejected += summary.rejected;
        total_errors += summary.errors;
        all_p50.push(summary.p50);
        all_p95.push(summary.p95);
        all_p99.push(summary.p99);
    }

    all_p50.sort_unstable();
    all_p95.sort_unstable();
    all_p99.sort_unstable();

    let median_p50 = if !all_p50.is_empty() {
        all_p50[all_p50.len() / 2]
    } else {
        0
    };
    let median_p95 = if !all_p95.is_empty() {
        all_p95[all_p95.len() / 2]
    } else {
        0
    };
    let median_p99 = if !all_p99.is_empty() {
        all_p99[all_p99.len() / 2]
    } else {
        0
    };

    let rate = total_submitted as f64 / args.duration as f64;

    println!("\nStress Test Summary:");
    println!("  Total submitted: {}", total_submitted);
    println!("  Total accepted:  {}", total_accepted);
    println!("  Total rejected:  {}", total_rejected);
    println!("  Total errors:    {}", total_errors);
    println!("  Avg rate:        {:.1} orders/sec", rate);
    println!("  Latency p50:     {} us", median_p50);
    println!("  Latency p95:     {} us", median_p95);
    println!("  Latency p99:     {} us", median_p99);

    Ok(())
}
