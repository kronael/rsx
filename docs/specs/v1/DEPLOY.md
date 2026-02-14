# Deployment Specification (v1)

## Table of Contents

- [Multi-Server Topology](#multi-server-topology)
- [Single-Machine Dev Topology](#single-machine-dev-topology)
- [Configuration](#configuration)
- [Security](#security)
- [Postgres HA](#postgres-ha)
- [Core Pinning](#core-pinning)
- [Monitoring](#monitoring)
- [Rolling Upgrades](#rolling-upgrades)
- [Backup](#backup)
- [Capacity Planning](#capacity-planning)
- [CMP/UDP Buffer Sizing](#cmpudp-buffer-sizing)
- [Health Endpoints](#health-endpoints)
- [Process Supervision](#process-supervision)
- [Log Rotation](#log-rotation)
- [Disk Layout](#disk-layout)

---

## Multi-Server Topology

[STUB] Production deployment across multiple machines.

```
┌──────────────────────────────────────┐
│  Gateway Tier (2+ hosts)             │
│    gw-0: Gateway + Marketdata        │
│    gw-1: Gateway + Marketdata        │
├──────────────────────────────────────┤
│  Matching Tier (1+ hosts)            │
│    me-0: ME BTC, ME ETH, ME SOL     │
│    me-1: ME PENGU, ME WIF, ME BONK  │
├──────────────────────────────────────┤
│  Risk Tier (1+ hosts)               │
│    risk-0: Risk primary (shard 0)    │
│    risk-1: Risk replica (shard 0)    │
├──────────────────────────────────────┤
│  Support Tier                        │
│    mark-0: Mark aggregator           │
│    rec-0: Recorder                   │
│    pg-0: Postgres primary            │
│    pg-1: Postgres replica            │
└──────────────────────────────────────┘
```

Gateway tier handles WS ingress and CMP fanout.
Matching tier runs one ME per symbol, pinned cores.
Risk tier runs sharded risk engines with replicas.

[STUB] Inter-tier networking: CMP/UDP on private VLAN,
WAL replication over TCP. Firewall rules TBD.

## Single-Machine Dev Topology

```
+-----------------------------------------+
|  Gateway (WS + CMP)                     |
|    +- Risk Engine (1 shard)             |
|         +- ME: PENGU  (core 2)         |
|         +- ME: SOL    (core 3)         |
|         +- ME: BTC    (core 4)         |
|                                         |
|  Mark Aggregator          (core 5)      |
|  Marketdata               (core 6)      |
|  Postgres                               |
+-----------------------------------------+
```

All processes on one host. Localhost UDP for CMP,
localhost TCP for WAL replication.

See `run.py` for automated local dev runner.

## Configuration

Environment variables with `RSX_` prefix per component.
No TOML config files. API keys via env vars.

### Matching Engine (per symbol)

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_ME_SYMBOL_ID | yes | - | Symbol ID (u32) |
| RSX_ME_PRICE_DECIMALS | yes | - | Price decimal places |
| RSX_ME_QTY_DECIMALS | yes | - | Qty decimal places |
| RSX_ME_TICK_SIZE | yes | - | Tick size (i64) |
| RSX_ME_LOT_SIZE | yes | - | Lot size (i64) |
| RSX_ME_WAL_DIR | no | ./tmp/wal | WAL directory |
| RSX_ME_CMP_ADDR | no | 127.0.0.1:9100 | ME CMP listen |
| RSX_RISK_CMP_ADDR | no | 127.0.0.1:9101 | Risk CMP addr |
| RSX_MD_CMP_ADDR | no | 127.0.0.1:9103 | Marketdata CMP |
| RSX_ME_DXS_ADDR | no | - | DXS sidecar addr |
| RSX_ME_DATABASE_URL | no | - | Postgres URL |
| RSX_ME_CORE_ID | no | - | CPU core to pin |

### Risk Engine

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_RISK_SHARD_ID | no | 0 | Shard ID |
| RSX_RISK_SHARD_COUNT | no | 1 | Total shards |
| RSX_RISK_MAX_SYMBOLS | no | 64 | Max symbols |
| RSX_RISK_CMP_ADDR | no | 127.0.0.1:9101 | Risk CMP |
| RSX_GW_CMP_ADDR | no | 127.0.0.1:9102 | Gateway CMP |
| RSX_ME_CMP_ADDR | no | 127.0.0.1:9100 | ME CMP addr |
| RSX_RISK_WAL_DIR | no | ./tmp/wal | WAL directory |
| RSX_RISK_REPLICA_ADDR | no | - | Replica addr |
| RSX_RISK_MARK_CMP_ADDR | no | 127.0.0.1:9105 | Mark CMP |
| RSX_MARK_CMP_ADDR | no | - | Mark sender |
| RSX_RISK_CORE_ID | no | - | CPU core to pin |
| DATABASE_URL | no | - | Postgres URL |

### Gateway

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_GW_LISTEN | no | 0.0.0.0:8080 | WS listen addr |
| RSX_GW_CMP_ADDR | no | 127.0.0.1:9102 | GW CMP addr |
| RSX_RISK_CMP_ADDR | no | 127.0.0.1:9101 | Risk CMP addr |
| RSX_GW_WAL_DIR | no | ./tmp/wal | WAL directory |
| RSX_GW_JWT_SECRET | no | dev-secret | JWT secret |
| RSX_GW_RL_USER | no | 10 | Rate limit/user |
| RSX_GW_RL_IP | no | 100 | Rate limit/IP |
| RSX_GW_RL_INSTANCE | no | 1000 | Rate limit total |

### Marketdata

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_MD_LISTEN | no | 0.0.0.0:8180 | WS listen addr |
| RSX_MKT_CMP_ADDR | no | 127.0.0.1:9103 | MKT CMP addr |
| RSX_ME_CMP_ADDR | no | 127.0.0.1:9100 | ME CMP addr |
| RSX_MD_STREAM_ID | no | 1 | Stream ID |

### Mark Aggregator

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_MARK_LISTEN_ADDR | no | 127.0.0.1:9400 | DXS listen |
| RSX_MARK_WAL_DIR | no | ./tmp/wal/mark | WAL dir |
| RSX_MARK_STREAM_ID | no | 100 | Stream ID |
| RSX_MARK_SYMBOL_MAP | no | "" | symbol=id,... |
| RSX_RISK_MARK_CMP_ADDR | no | 127.0.0.1:9105 | Risk mark |

### Recorder

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_RECORDER_STREAM_ID | yes | - | Stream to record |
| RSX_RECORDER_PRODUCER_ADDR | yes | - | DXS source |
| RSX_RECORDER_ARCHIVE_DIR | yes | - | Archive dir |
| RSX_RECORDER_TIP_FILE | yes | - | Tip file path |

## Security

[STUB] Production security requirements.

- TLS between hosts for CMP replication (TCP path)
- CMP/UDP on private VLAN, not exposed externally
- JWT rotation: key rotation interval TBD
- Firewall: only gateway tier exposed to internet
- Postgres: TLS required, scram-sha-256 auth
- WAL files: filesystem permissions 0600
- No secrets in env files on shared hosts (use vault)

## Postgres HA

[STUB] High-availability Postgres setup.

- Primary + streaming replica
- pgbouncer in front (transaction pooling)
- Connection string via DATABASE_URL env var
- Failover: manual promotion of replica (v1)
- Automated failover via patroni (v2)

## Core Pinning

[STUB] CPU core assignment strategy.

Single machine (8+ cores):
- Core 0-1: OS + Postgres
- Core 2-4: ME instances (one per core)
- Core 5: Mark aggregator
- Core 6: Risk engine
- Core 7: Gateway + Marketdata

Multi-machine: each ME gets a dedicated core,
risk and gateway get dedicated cores per host.

Set via RSX_*_CORE_ID env vars. Use `isolcpus`
kernel param to prevent OS scheduling on hot cores.

## Monitoring

[STUB] Observability stack.

- Structured JSON logs to stdout (tracing crate)
- Log shipping: journald → vector → clickhouse
- Health polling: HTTP /health every 5s
- Metrics: structured log lines, not Prometheus
- Alerting: seq gap > threshold, health 503, OOM
- Dashboard: Grafana reading from clickhouse

## Rolling Upgrades

[STUB] Zero-downtime upgrade procedure.

1. Build new binary, verify on staging
2. Drain gateway (stop accepting new connections)
3. Wait for in-flight orders to complete (timeout 30s)
4. Stop old binary, start new binary
5. Gateway reconnects, resumes accepting connections
6. Verify health + seq continuity

ME upgrade requires WAL replay from last snapshot.
Risk upgrade requires Postgres state + WAL replay.

## Backup

[STUB] Backup and archival strategy.

- WAL archival: recorder writes daily files
- Postgres: pg_dump daily, WAL archiving continuous
- Retention: WAL 30 days, pg_dump 90 days
- Offsite: S3-compatible object storage
- Recovery: restore pg_dump + replay WAL from tip

## Capacity Planning

[STUB] Resource estimation.

| Users | Symbols | ME Cores | Risk Cores | RAM |
|---|---|---|---|---|
| 1K | 3 | 3 | 1 | 4GB |
| 10K | 8 | 8 | 2 | 16GB |
| 100K | 8 | 8 | 4 | 64GB |

Disk: ~1GB/day/symbol WAL at 1K orders/s.
Network: ~100Mbps per ME at peak.

## CMP/UDP Buffer Sizing

CMP/UDP uses kernel socket buffers. Tune via sysctl:

```
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
```

Default kernel buffers adequate for v1. Monitor
CMP NAK rate before increasing.

## Health Endpoints

Every component exposes `/health` on its listen addr:

```json
{
  "status": "ok",
  "seq": 1234567,
  "version": "0.1.0",
  "uptime_sec": 3600
}
```

- `200 OK`: ready and processing
- `503 Service Unavailable`: starting or degraded

## Process Supervision

Systemd unit files per component. Restart=always
with 1s delay. ME units require CPUAffinity=.

## Log Rotation

Structured JSON logs to stdout. Systemd journal
handles rotation. File-based: logrotate daily,
7-day retention.

## Disk Layout

```
/srv/data/rsx/
  wal/          # WAL files per stream
  snapshot/     # ME snapshots per symbol
  archive/      # Recorder daily archives
  log/          # Debug/smoke logs
```
