# Deployment Specification (v1)

Single-machine development topology. Production deployment is
a superset (same components, distributed across machines).

## v1 Targets

- 1 matching engine per symbol
- 1 risk shard (single user partition)
- <10K concurrent users
- <10 symbols

## Single-Machine Topology

```
+-----------------------------------------+
|  Gateway (WS + gRPC)                    |
|    +- Risk Engine (1 shard)             |
|         +- ME: BTC-PERP  (core 2)      |
|         +- ME: ETH-PERP  (core 3)      |
|         +- ME: SOL-PERP  (core 4)      |
|                                         |
|  Mark Aggregator          (core 5)      |
|  MARKETDATA               (core 6)      |
|  Postgres                               |
+-----------------------------------------+
```

All processes on one host. UDS for IPC where applicable,
localhost TCP otherwise.

## Configuration

Environment files with `RSX_` prefix per component:

```
# /etc/rsx/env/me-btc.env
RSX_ME_SYMBOL_ID=1
RSX_ME_LISTEN_ADDR=127.0.0.1:9001
RSX_ME_WAL_DIR=./wal/btc
RSX_ME_SNAPSHOT_DIR=./snapshot/btc
RSX_ME_CORE_AFFINITY=2

# /etc/rsx/env/risk.env
RSX_RISK_ADDR=127.0.0.1:9010
RSX_RISK_SHARD_ID=0
RSX_RISK_POSTGRES_URL=postgres://localhost:5432/rsx
RSX_RISK_ME_ADDRS=127.0.0.1:9001,127.0.0.1:9002

# /etc/rsx/env/gateway.env
RSX_GATEWAY_WS_ADDR=0.0.0.0:8080
RSX_GATEWAY_GRPC_ADDR=0.0.0.0:8081
RSX_GATEWAY_RISK_ADDR=127.0.0.1:9010

# /etc/rsx/env/mark.env
RSX_MARK_LISTEN_ADDR=127.0.0.1:9200
RSX_MARK_WAL_DIR=./wal/mark
```

TOML config file as first CLI argument overrides env vars.
API key file as second CLI argument.

## SPSC Ring Sizing

Ring capacity = `peak_throughput * 2` (headroom factor).

| Ring | Capacity | Rationale |
|------|----------|-----------|
| ME -> Risk (fills) | 4096 | ~1ms at 4M fills/s |
| ME -> Gateway | 4096 | same as Risk |
| ME -> MARKETDATA | 8192 | lower priority, more lag |
| Gateway -> Risk | 4096 | order ingress |
| Risk -> ME | 2048 | validated orders |
| Mark -> Risk | 1024 | mark prices (low rate) |

Resize if monitoring shows >10 stalls/sec sustained.

## Health Endpoints

Every component exposes `/health` on its listen address:

```json
{
  "status": "ok",
  "seq": 1234567,
  "version": "0.1.0",
  "uptime_sec": 3600
}
```

- `200 OK`: component is ready and processing
- `503 Service Unavailable`: component is starting, replaying,
  or degraded (not yet ready to serve)
- `seq`: latest processed sequence number (0 if N/A)
- `version`: binary version string
- `uptime_sec`: seconds since process start

## Process Supervision

Systemd unit files per component. Restart policy: `always`
with 1s delay. Matching engines require `CPUAffinity=` for
core pinning.

## Log Rotation

Structured JSON logs to stdout. Systemd journal handles
rotation. For file-based logging: logrotate with daily
rotation, 7-day retention.

## Disk Layout

```
/srv/data/rsx/
  wal/          # WAL files per stream
  snapshot/     # ME snapshots per symbol
  archive/      # Recorder daily archives
  log/          # Debug/smoke logs
```
