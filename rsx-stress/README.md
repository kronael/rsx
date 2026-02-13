# rsx-stress

WebSocket stress test client for RSX Gateway.

## Build

```
cargo build -p rsx-stress --release
```

## Usage

Basic usage (1000 orders/sec for 60 seconds):

```
./target/release/rsx-stress --gateway ws://localhost:8080
```

Custom configuration:

```
./target/release/rsx-stress \
  --gateway ws://localhost:8080 \
  --rate 5000 \
  --duration 120 \
  --users 100 \
  --connections 50 \
  --output stress-results.csv
```

## Options

- `--gateway`: WebSocket URL (default: ws://localhost:8080)
- `--rate`: Target orders/second (default: 1000)
- `--duration`: Test duration in seconds (default: 60)
- `--users`: Number of user IDs to distribute across (default: 10)
- `--connections`: Concurrent WebSocket connections (default: 10)
- `--output`: CSV output file for latency data (default: stress-test.csv)

## Output

Summary printed to stdout:
```
Stress Test Summary:
  Total submitted: 60000
  Total accepted:  59980
  Total rejected:  15
  Total errors:    5
  Avg rate:        1000.0 orders/sec
  Latency p50:     850 us
  Latency p95:     2100 us
  Latency p99:     4500 us
```

CSV output contains per-order latency:
```
timestamp,oid,latency_us,status
1707856234,01234...,850,accepted
```

## Testing

```
cargo test -p rsx-stress
```

## Architecture

- WebSocket client with tokio-tungstenite
- Concurrent workers (1 WebSocket per worker)
- Rate limiting via tokio interval timer
- HDR histogram for latency tracking
- Order distribution: 50% BTC, 30% ETH, 20% SOL
- 50/50 buy/sell split
- Random prices within ±1% of mid
- Unique client_order_id per order (timestamp + counter)
