---
status: partial
---

# Deployment Specification (v1)

## Table of Contents

- [Multi-Server Topology](#multi-server-topology)
- [Single-Machine Topology](#single-machine-topology)
- [Configuration](#configuration)
- [Security](#security)
- [Postgres HA](#postgres-ha)
- [Core Pinning](#core-pinning)
- [Monitoring](#monitoring)
- [Rolling Upgrades](#rolling-upgrades)
- [Backup](#backup)
- [Capacity Planning](#capacity-planning)
- [casting/UDP Buffer Sizing](#cmpudp-buffer-sizing)
- [Health Endpoints](#health-endpoints)
- [Process Supervision](#process-supervision)
- [Log Rotation](#log-rotation)
- [Disk Layout](#disk-layout)

---

## Multi-Server Topology

[STUB — founder-owned] Production deployment across multiple machines
depends on host count and tiering decisions the founder owns (how many
gateway/matching/risk hosts, where replicas live, the private VLAN
layout). It is deliberately left as a stub. The **single-machine**
deploy below is fully specified and is the first production deploy;
scale out to this topology later.

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

Gateway tier handles WS ingress and casting fanout.
Matching tier runs one ME per symbol, pinned cores.
Risk tier runs sharded risk engines with replicas.

[STUB] Inter-tier networking: casting/UDP on private VLAN,
WAL replication over TCP. Firewall rules TBD.

## Single-Machine Topology

All processes on one host. Casting over loopback UDP, WAL replication
over loopback TCP, one Postgres. This is both the dev runner and the
first production deploy — same wiring, different bind/secret/disk
posture.

**Dev**: `./rsx-playground/playground start-all <scenario>` (or the
repo-root `start` script) launches the cluster in-memory. Scenarios in
`start` (`minimal`, `full`, `stress`, …) pick the symbol set, gateway
count, and replica posture.

**Production**: systemd units under `deploy/`, one command. Symbols
BTC (id 1), ETH (id 2), SOL (id 3); risk shard 0; one gateway, one
marketdata, mark, and one recorder per symbol. No risk replicas — a
same-box replica shares the failure domain, so durability comes from
the WAL + per-symbol recorder archive + Postgres, not replication.

```
+--------------------------------------------------+
|  core 0  OS + mark + recorders (off-path)        |
|  core 1  gateway        (WS :8080, io_uring)     |
|  core 2  risk shard 0   (cast :9200)             |
|  core 3  ME BTC (id 1)  (cast :9101, repl :9701) |
|  core 4  ME ETH (id 2)  (cast :9102, repl :9702) |
|  core 5  ME SOL (id 3)  (cast :9103, repl :9703) |
|  core 6  marketdata     (WS :8180, io_uring)     |
|  core 7  spare / Postgres / nginx                |
+--------------------------------------------------+
```

Order path: client → nginx(TLS) → GW :8080 → risk :9200 → ME :91xx →
risk → GW → client. Public market data: ME :91xx → marketdata :8180 →
nginx → subscribers. Only 8080/8180 are proxied out; every casting and
replication port stays on 127.0.0.1.

The concrete units, per-instance env, and the one-command installer
live in `deploy/` (see `deploy/README.md`). The tables below are the
reference; `deploy/env/*.env` is the authoritative wiring.

## Configuration

Environment variables with `RSX_` prefix per component.
No TOML config files. API keys via env vars.

The single-machine production values are baked into
`deploy/env/*.env` (one `EnvironmentFile` per instance); `${PREFIX}`
and `${DATABASE_URL}` are substituted at deploy time. The gateway JWT
secret is the one exception — it lives in `deploy/env/secret.env`
(mode 0400), loaded as a second `EnvironmentFile`, never in a unit or
the main env file. The tables below list the vars each binary reads;
addresses default to loopback for single-machine.

### Matching Engine (per symbol)

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_ME_SYMBOL_ID | yes | - | Symbol ID (u32) |
| RSX_ME_PRICE_DECIMALS | yes | - | Price decimal places |
| RSX_ME_QTY_DECIMALS | yes | - | Qty decimal places |
| RSX_ME_TICK_SIZE | yes | - | Tick size (i64) |
| RSX_ME_LOT_SIZE | yes | - | Lot size (i64) |
| RSX_ME_WAL_DIR | no | ./tmp/wal | WAL directory |
| RSX_ME_CAST_ADDR | no | 127.0.0.1:9100 | ME casting listen |
| RSX_RISK_CAST_ADDR | no | 127.0.0.1:9101 | Risk casting addr |
| RSX_MD_CAST_ADDR | no | 127.0.0.1:9103 | Marketdata casting |
| RSX_ME_REPLICATION_BIND_ADDR | no | - | replication sidecar addr |
| RSX_ME_DATABASE_URL | no | - | Postgres URL |
| RSX_ME_CORE_ID | no | - | CPU core to pin |
| RSX_ME_HEALTH_ADDR | no | - | Health/metrics listen |

### Risk Engine

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_RISK_SHARD_ID | no | 0 | Shard ID |
| RSX_RISK_SHARD_COUNT | no | 1 | Total shards |
| RSX_RISK_MAX_SYMBOLS | no | 64 | Max symbols |
| RSX_RISK_CAST_ADDR | no | 127.0.0.1:9200 | Risk casting listen |
| RSX_GW_CAST_ADDR | no | 127.0.0.1:9300 | Gateway casting |
| RSX_ME_CAST_ADDRS | no | 127.0.0.1:9100 | ME casting addrs (comma list) |
| RSX_ME_REPLICATION_ADDR | no | - | ME replication addrs (FAULTED replay) |
| RSX_RISK_WAL_DIR | no | ./tmp/wal | WAL directory |
| RSX_RISK_MARK_CAST_ADDR | no | 127.0.0.1:9600 | Mark casting listen |
| RSX_MARK_CAST_ADDR | no | - | Mark sender addr |
| RSX_RISK_CORE_ID | no | - | CPU core to pin |
| RSX_RISK_HEALTH_ADDR | no | - | Health/metrics listen |
| DATABASE_URL | no | - | Postgres URL |

### Gateway

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_GW_LISTEN | no | 0.0.0.0:8080 | WS listen addr |
| RSX_GW_CAST_ADDR | no | 127.0.0.1:9102 | GW casting addr |
| RSX_RISK_CAST_ADDR | no | 127.0.0.1:9101 | Risk casting addr |
| RSX_GW_WAL_DIR | no | ./tmp/wal | WAL directory |
| RSX_GW_JWT_SECRET | yes | (panic) | HMAC-SHA256 JWT signing secret. Production must override the dev secret used by `start`. |
| RSX_GW_RL_USER | no | 10 | Rate limit/user |
| RSX_GW_RL_IP | no | 100 | Rate limit/IP |
| RSX_GW_CORE_ID | no | - | CPU core to pin |
| RSX_GW_HEALTH_ADDR | no | - | Health/metrics listen |
| RSX_SYMBOL_<id>_TICK_SIZE / _LOT_SIZE | no | - | Per-symbol tick/lot |

### Marketdata

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_MD_LISTEN | no | 0.0.0.0:8180 | WS listen addr |
| RSX_ME_CAST_ADDRS | no | 127.0.0.1:9100 | ME casting addrs (comma list) |
| RSX_MD_STREAM_ID | no | 1 | Stream ID (first ME sid) |
| RSX_MD_REPLAY_ADDR | no | - | ME replication addr for replay |
| RSX_MD_CORE_ID | no | - | CPU core to pin |
| RSX_MD_HEALTH_ADDR | no | - | Health/metrics listen |

### Mark Aggregator

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_MARK_LISTEN_ADDR | no | 127.0.0.1:9400 | replication listen |
| RSX_MARK_WAL_DIR | no | ./tmp/wal/mark | WAL dir |
| RSX_MARK_STREAM_ID | no | 100 | Stream ID |
| RSX_MARK_SYMBOL_MAP | no | "" | symbol=id,... |
| RSX_RISK_MARK_CAST_ADDR | no | 127.0.0.1:9600 | Risk mark casting |
| RSX_MARK_SOURCE_BINANCE_ENABLED | no | 0 | Enable Binance feed |
| RSX_MARK_SOURCE_BINANCE_WS_URL | no | - | Binance combined-stream URL |
| RSX_MARK_HEALTH_ADDR | no | - | Health/metrics listen |

### Recorder

| Variable | Required | Default | Description |
|---|---|---|---|
| RSX_RECORDER_STREAM_ID | yes | - | Stream to record |
| RSX_RECORDER_PRODUCER_ADDR | yes | - | replication source |
| RSX_RECORDER_ARCHIVE_DIR | yes | - | Archive dir (dedicated volume) |
| RSX_RECORDER_TIP_FILE | yes | - | Tip file path |
| RSX_RECORDER_RETAIN_DAYS | no | 3 | Local rolling-window prune (days) |
| RSX_RECORDER_HEALTH_ADDR | no | - | Health/metrics listen |

## Security

Single-machine posture (the one deployed today):

- **TLS terminates at nginx/caddy** in front of the gateway. The
  gateway binds `127.0.0.1:8080` and marketdata `127.0.0.1:8180`; the
  proxy exposes `wss://rsx.krons.cx` and the public market-data WS.
  The RSX binaries never terminate TLS.
- **casting/UDP is internal-only** — every casting and replication
  port binds loopback (`127.0.0.1`), so on one host it never leaves the
  box. This is the trust boundary from `specs/2/4-cast.md` §10.4
  (trusted internal network, no per-frame auth); auth lives at the
  gateway (JWT) and the network edge (firewall). Inbound firewall:
  443 (proxy) + 22 (ssh) only.
- **Gateway JWT** (`RSX_GW_JWT_SECRET`) is HMAC-SHA256, shared with the
  auth service that mints client tokens. It lives in
  `/opt/rsx/env/secret.env` (mode 0400, owned `rsx`), loaded as its own
  `EnvironmentFile` — never committed, never in a unit. `deploy.sh`
  refuses to start if it is missing or still the placeholder.
- **Postgres**: `scram-sha-256` auth; on a single box it listens on
  loopback, so TLS to the DB is optional (add it if the DB moves off
  the host). `DATABASE_URL` is passed via env, not baked into units.
- **Filesystem**: env files 0640, secret 0400, data dirs 0750 owned by
  the `rsx` system user. Units run as `User=rsx` with
  `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`,
  `PrivateTmp`, and `ReadWritePaths=` scoped to the data root.

[STUB — founder-owned] Cross-host hardening (mutual TLS on the
replication TCP path, casting on a private VLAN, JWT rotation cadence,
a secrets vault instead of on-disk files) belongs to the multi-server
topology and is decided when it lands.

## Postgres HA

Single machine: one local Postgres, `scram-sha-256`, listening on
loopback, exposed to systemd as `rsx-postgres.service` (the RSX units
`Requires=`/`After=` it — it must come up first, or risk crash-loops on
connect; see `bugs.md` STARTUP-ORDERING-FRAGILITY). `DATABASE_URL`
carries the DSN. No replica on the same box (same failure domain).

[STUB — founder-owned] High-availability Postgres (below) lands with
the multi-server topology.

- Primary + streaming replica
- pgbouncer in front (transaction pooling)
- Connection string via DATABASE_URL env var
- Failover: manual promotion of replica (v1)
- Automated failover via patroni (v2)

## Core Pinning

Every order-path process busy-spins (`specs/2/45-tiles.md`) and MUST
own a core. An unpinned spinner floats across cores under CFS load
balancing, lands on a hot core, starves the consumer, overflows its UDP
socket → `RcvbufErrors` → dropped packets → a FAULTED replay storm. So
pinning is a correctness property, not just tuning.

Single machine (8-core, BTC/ETH/SOL):

| Core | Process | Path |
|---|---|---|
| 0 | OS + mark + recorders | off-path (must not busy-spin) |
| 1 | gateway | hot |
| 2 | risk shard 0 | hot |
| 3 | ME BTC (id 1) | hot |
| 4 | ME ETH (id 2) | hot |
| 5 | ME SOL (id 3) | hot |
| 6 | marketdata | hot (off the order round-trip) |
| 7 | spare / Postgres / nginx | — |

Two layers enforce it: `RSX_*_CORE_ID` makes the app pin itself
(`core_affinity`), and `CPUAffinity=` in the systemd unit bounds the
cpuset at the OS level. The templated `rsx-me@` unit sets
`CPUAffinity=3 4 5` (the ME pool) and each instance narrows to its one
core via `RSX_ME_CORE_ID`. Off-path units pin to core 0. For hard
isolation add `isolcpus=1-6 nohz_full=1-6 rcu_nocbs=1-6` to the kernel
cmdline so the OS never schedules other work onto the hot cores.

Adding a symbol takes the next free core (6→marketdata shifts up, or
move to a larger box). More than 3 symbols on 8 cores oversubscribes —
scale to the multi-server topology.

[STUB — founder-owned] Multi-machine core assignment per host.

## Monitoring

- Each daemon runs `spawn_health_server` (rsx-health) on its
  `RSX_*_HEALTH_ADDR` (98xx band, loopback), off the hot path. Endpoints
  and ports are in "Health Endpoints" below.
- Structured logs to stdout (tracing, `RUST_LOG=info`), captured by the
  systemd journal per unit: `journalctl -u 'rsx-*' -f`.
- Metrics are structured log lines + the `/metrics` JSON snapshot, not
  Prometheus (per repo convention). A separate reader ships them
  onward; on one box, polling `/metrics` every 5 s is enough.
- Health polling: hit `/health` (liveness) and `/ready` (readiness) on
  each daemon. `deploy.sh --apply` runs the full `/health` sweep after
  start and fails if any endpoint stays down 30 s.
- Alert on: `/health` 503, seq gap / FAULTED replay in the logs, UDP
  `RcvbufErrors` (casting overrun), OOM, archive-volume free space.

[STUB — founder-owned] Central log shipping + dashboards (journald →
vector → store → Grafana) land with the multi-server topology.

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

## casting/UDP Buffer Sizing

The order firehose (ME → risk, ME → marketdata) is casting/UDP. The
default ~208 KB kernel receive buffer overruns under load: the kernel
drops datagrams → `RcvbufErrors` → the consumer sees a seq gap → it
FAULTS into replay. Raise the buffers to 25 MiB via sysctl:

```
net.core.rmem_max = 26214400
net.core.wmem_max = 26214400
net.core.rmem_default = 26214400
```

`deploy.sh` installs this as `/etc/sysctl.d/99-rsx.conf`
(`deploy/sysctl/99-rsx.conf`) and runs `sysctl --system`; `make
tune-host` applies the same values in dev. Monitor `RcvbufErrors`
(`nstat -az | grep Udp`) and the casting NAK rate; increase further
only if drops persist under peak.

## Health Endpoints

Every daemon binds a small HTTP server (rsx-health) on its
`RSX_*_HEALTH_ADDR`, off the hot path. Three endpoints:

- `GET /health` → `200 {"status":"ok"}` when live, else
  `503 {"status":"not_live"}` — liveness (restart on 503).
- `GET /ready` → `200 {"status":"ready"}` when ready, else
  `503 {"status":"not_ready"}` — readiness (shed traffic on 503).
- `GET /metrics` (alias `/loadz`) → `200` + a JSON load snapshot.

Single-machine health ports (loopback):

| Process | Health addr |
|---|---|
| ME BTC / ETH / SOL | 127.0.0.1:9801 / :9802 / :9803 |
| risk shard 0 | 127.0.0.1:9900 |
| gateway | 127.0.0.1:10000 |
| mark | 127.0.0.1:10100 |
| marketdata | 127.0.0.1:10200 |
| recorder BTC / ETH / SOL | 127.0.0.1:10301 / :10302 / :10303 |

## Process Supervision

systemd, one unit per process, under `deploy/systemd/`. The sharded
processes are templated: `rsx-me@` (instance = symbol slug, e.g.
`rsx-me@btc`), `rsx-risk@` (instance = shard), `rsx-recorder@`
(instance = symbol). `rsx-gateway`, `rsx-marketdata`, `rsx-mark` are
single units. `rsx.target` aggregates them for one-shot start/stop.

- `Restart=on-failure`, `RestartSec=1` (2 for off-path). A crash-loop
  guard (`StartLimitIntervalSec=60`, `StartLimitBurst=5`) stops a unit
  that fails 5×/60 s instead of masking a real fault (Postgres down,
  port taken) behind infinite restarts — see `bugs.md`
  STARTUP-ORDERING-FRAGILITY.
- Ordering (`After=`/`Requires=`): `rsx-postgres.service` → ME → risk;
  ME before marketdata (marketdata replays the ME stream on connect and
  panics if the ME replication server isn't up) and before the recorder
  (same replication source). Gateway after risk. mark after
  `network-online` (it reaches external CEX feeds).
- No `sd_notify`/`WatchdogSec` — the binaries don't emit readiness to
  systemd; readiness is observed via the `/health` and `/ready`
  endpoints (deploy.sh gates on `/health`, external monitoring polls
  both). `Type=simple`.
- `CPUAffinity=` per unit for pinning (see Core Pinning), plus
  `LimitMEMLOCK=infinity` on the io_uring processes (gateway,
  marketdata) and a high `LimitNOFILE` on the gateway for many WS fds.

## Log Rotation

Structured logs to stdout, captured per unit by the systemd journal —
`journalctl` handles rotation and retention (size/time caps via
`journald.conf`). No app-side log files in production. Dev debug/smoke
logs go to `${PREFIX}/data/rsx/log/`.

## Disk Layout

```
/opt/rsx/
  bin/          # release binaries (rsx-matching, rsx-risk, …)
  env/          # per-instance EnvironmentFiles + secret.env (0400)
  specs/        # deployed copy of specs/2/9-deploy.md (unit docs link)

${PREFIX:-/srv}/data/rsx/
  wal/          # hot-tier WAL, 4h retention
    btc/ eth/ sol/   # one dir per ME symbol
    mark/            # mark WAL
  snapshot/     # ME snapshots per symbol
  log/          # dev debug/smoke logs
  recorder-tip-<sid>  # recorder replay tip per stream

${PREFIX:-/srv}/data/rsx/archive/   # DEDICATED VOLUME — mount first
  <sid>/        # recorder audit stream per symbol (unbounded by design)
```

**The archive must be its own volume.** The recorder writes the full
ME stream here as the permanent audit / replay-from-genesis tier — it
grows without bound (hours of maker churn = tens of GB). Never let it
share the root fs or it ENOSPCs the box (`bugs.md`
RECORDER-ARCHIVE-DEV-DISK / FINDINGS #28). `RSX_RECORDER_RETAIN_DAYS`
prunes only the local rolling window; the permanent tier is a daily
object-store offload (S3/GCS, founder-owned cron). Retain locally
longer than the offload cadence so nothing is pruned before it ships.
