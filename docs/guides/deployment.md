# Deployment Guide

This guide covers deploying RSX to production environments.

## Overview

RSX is designed to run as separate processes with UDP/TCP communication between them. Each process should run on dedicated hardware with pinned CPU cores for optimal performance.

## Hardware Requirements

### Minimum (Development)
- 4 cores
- 8 GB RAM
- SSD storage
- 1 Gbps network

### Recommended (Production)
- 16+ cores (dedicated per process)
- 32+ GB RAM
- NVMe SSD storage
- 10 Gbps network (25 Gbps for high-frequency trading)

## Process Layout

Each component runs as a separate binary with dedicated resources:

```
[Gateway]     → Core 0-1, 4 GB RAM, WS connections
[Risk]        → Core 2-3, 8 GB RAM, margin calculations
[Matching-BTC] → Core 4, 2 GB RAM, orderbook matching
[Matching-ETH] → Core 5, 2 GB RAM, orderbook matching
[MarketData]  → Core 6-7, 4 GB RAM, shadow book + broadcast
[Mark]        → Core 8, 2 GB RAM, external feeds
[Recorder]    → Core 9, 4 GB RAM, WAL archival
```

## Configuration

### Environment Variables

```bash
export DATABASE_URL="postgres://user:pass@host:5432/rsx"
export JWT_SECRET="production-secret-key"
export RISK_UDP_ADDR="10.0.1.10:9000"
export ME_BTC_UDP_ADDR="10.0.1.11:9001"
export ME_ETH_UDP_ADDR="10.0.1.12:9002"
export GATEWAY_WS_ADDR="0.0.0.0:8080"
```

### TOML Configuration

Each process reads TOML config from first CLI argument:

```toml
# gateway.toml
[gateway]
ws_addr = "0.0.0.0:8080"
risk_udp_addr = "10.0.1.10:9000"
jwt_secret_env = "JWT_SECRET"
rate_limit_per_sec = 100
circuit_breaker_threshold = 50

[logging]
level = "info"
format = "json"
```

See [specs/v1/DEPLOY.md](../specs/v1/DEPLOY.md) for complete configuration options.

## Deployment Steps

### 1. Build Release Binaries

```bash
cargo build --release --workspace
```

Binaries will be in `target/release/`:
- `rsx-gateway`
- `rsx-risk`
- `rsx-matching`
- `rsx-marketdata`
- `rsx-mark`
- `rsx-recorder`

### 2. Prepare Directories

```bash
mkdir -p /srv/data/rsx/wal
mkdir -p /srv/data/rsx/snapshots
mkdir -p /var/log/rsx
mkdir -p /var/run/rsx
```

### 3. Install Systemd Units

```ini
[Unit]
Description=RSX Gateway
After=network.target

[Service]
Type=simple
User=rsx
Group=rsx
WorkingDirectory=/srv/rsx
ExecStart=/srv/rsx/bin/rsx-gateway /srv/rsx/config/gateway.toml
Restart=always
RestartSec=10
CPUAffinity=0 1

[Install]
WantedBy=multi-user.target
```

### 4. Start Services

```bash
systemctl start rsx-risk
systemctl start rsx-matching@btc
systemctl start rsx-matching@eth
systemctl start rsx-gateway
systemctl start rsx-marketdata
systemctl start rsx-mark
systemctl start rsx-recorder
```

## Monitoring

See [Monitoring Guide](monitoring.md) for metrics, alerts, and dashboards.

## Recovery

See [Operations Guide](operations.md) for crash recovery procedures.

## Security

### Network

- Gateway WS should be behind TLS termination (nginx/haproxy)
- Internal CMP/UDP should be on isolated network
- PostgreSQL should not be publicly accessible

### Authentication

- JWT tokens with short expiration (1 hour)
- API keys for service-to-service communication
- Rate limiting per user (100 req/sec default)

### WAL

- WAL files contain all order data (PII)
- Encrypt at rest
- Rotate and archive daily
- Retain 30 days minimum for recovery

## Performance Tuning

### CPU Pinning

```bash
taskset -c 4 ./rsx-matching config.toml
```

### Network Tuning

```bash
# Increase UDP buffer sizes
sysctl -w net.core.rmem_max=134217728
sysctl -w net.core.wmem_max=134217728
```

### Storage

- Use NVMe SSD for WAL writes
- Separate disk for PostgreSQL
- Consider io_uring for Gateway/MarketData (already used)

## Troubleshooting

### High Latency

1. Check CPU pinning: `ps -eLo pid,tid,psr,comm | grep rsx`
2. Check network: `netstat -su` for UDP drops
3. Check WAL flush: log `wal_flush_lag_ms` metric

### Lost Orders

1. Check WAL integrity: `rsx-cli wal dump /srv/data/rsx/wal/`
2. Check CMP flow control: log `cmp_backpressure_events`
3. Check SPSC ring full: log `ring_full_stalls`

### Crashes

1. Follow [Recovery Runbook](operations.md)
2. Check logs: `journalctl -u rsx-risk`
3. Replay WAL: `rsx-cli wal replay /srv/data/rsx/wal/`
