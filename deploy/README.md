# RSX single-machine production deploy

Stand the whole RSX cluster up on **one host** (e.g. `rsx.krons.cx`)
under systemd. This is the natural first production deploy: every
process on one box, casting over loopback, one Postgres. The
multi-server topology is a separate, founder-owned decision — see
`specs/2/9-deploy.md` "Multi-Server Topology".

Symbols deployed: **BTC (1), ETH (2), SOL (3)**. Risk shard 0, one
gateway, one marketdata, mark, and one recorder per symbol.

## What runs where

| Unit | Binary | Core | Listens (loopback) | Health |
|---|---|---|---|---|
| `rsx-me@btc/eth/sol` | rsx-matching | 3 / 4 / 5 | cast 9101/2/3 | 9801/2/3 |
| `rsx-risk@0` | rsx-risk | 2 | cast 9200 | 9900 |
| `rsx-gateway` | rsx-gateway | 1 | **ws 8080** | 10000 |
| `rsx-marketdata` | rsx-marketdata | 6 | **ws 8180** | 10200 |
| `rsx-mark` | rsx-mark | 0 | cast 9400 | 10100 |
| `rsx-recorder@btc/eth/sol` | rsx-recorder | 0 | — | 10301/2/3 |

Core 0 is the OS + off-path (mark, recorder). Core 7 is spare
(Postgres, nginx). All order-path processes busy-spin and own a core;
`CPUAffinity=` in the units plus `RSX_*_CORE_ID` in the env pin them.
See `specs/2/9-deploy.md` "Core Pinning".

## Prerequisites (founder-owned, do first)

1. **Host**: 8+ physical cores, Linux with systemd. For hard isolation
   of the hot cores add `isolcpus=1-6` (and `nohz_full=1-6
   rcu_nocbs=1-6`) to the kernel cmdline and reboot.
2. **Postgres** reachable as `DATABASE_URL` (default
   `postgres://rsx@127.0.0.1:5432/rsx`). It must come up **before** the
   cluster and be exposed to systemd as **`rsx-postgres.service`** (the
   units `Requires=`/`After=` it). If you run Postgres in Docker, wrap
   it: create `/etc/systemd/system/rsx-postgres.service` that
   `ExecStart=/usr/bin/docker start -a rsx-postgres` (and
   `docker update --restart unless-stopped rsx-postgres`), or install
   the distro `postgresql` package and symlink the alias. Schema:
   apply `rsx-auth/` migrations and seed collateral accounts.
3. **Archive volume**: mount a **dedicated disk** at
   `${PREFIX:-/srv}/data/rsx/archive` before first run. The recorder
   archive is the exchange's audit / replay-from-genesis tier and is
   **unbounded by design** — never point it at the root fs (it will
   ENOSPC the box). See "Archive offload" below.
4. **Edge TLS + DNS** (client-facing): point `rsx.krons.cx` at the
   host; terminate TLS at nginx/caddy and reverse-proxy
   `wss://rsx.krons.cx` → gateway `127.0.0.1:8080` and the public
   market-data WS → `127.0.0.1:8180`. casting/replication ports stay
   on loopback and are never exposed (firewall inbound to 443/22
   only).
   - **Internal replication TLS** is separate and **mandatory**:
     `deploy.sh` auto-provisions snakeoil certs into
     `${PREFIX:-/srv}/data/rsx/certs` and the env files point
     `RSX_REPL_CERT_PATH` / `RSX_REPL_KEY_PATH` / `RSX_REPL_CA_PATH`
     at them. For real prod, replace those PEM files with proper
     certs (same paths) — regenerate any time with
     `RSX_REPL_CERT_DIR=/srv/data/rsx/certs sh
     scripts/gen-snakeoil-certs.sh --force`. The casting/UDP order
     path stays plaintext by design (trusted LAN, spec 4-cast §10.4).
5. **JWT secret**: mint one and stage it (never committed, never in a
   unit):
   ```
   cp deploy/env/secret.env.example /opt/rsx/env/secret.env
   printf 'RSX_GW_JWT_SECRET=%s\n' "$(openssl rand -hex 32)" \
     > /opt/rsx/env/secret.env
   install -m 0400 -o rsx -g rsx /opt/rsx/env/secret.env /opt/rsx/env/secret.env
   ```
   Use the same secret in the auth service that mints client tokens.

## The one command

On the target, as root, from the repo root:

```
sudo RSX_DEPLOY_HOST="$(hostname -f)" ./deploy/deploy.sh --apply
```

Without `--apply` it is a **dry run** — it prints every action and
changes nothing. `RSX_DEPLOY_HOST` must match the box's FQDN; the
script refuses to apply otherwise (so you can't run it on the wrong
host). It builds `--release`, installs binaries to `/opt/rsx/bin`,
stages env to `/opt/rsx/env`, applies the sysctl tuning, installs +
enables the units, starts `rsx.target`, and polls every `/health`.

Override the data root with `PREFIX=/mnt/rsx` and the DB with
`DATABASE_URL=...` in the environment.

This repo does **not** deploy for you: DNS, TLS, git push, and the
object-store offload are intentionally the founder's to run.

## Verify

```
systemctl status rsx.target
systemctl list-dependencies rsx.target
curl -fsS http://127.0.0.1:10000/health   # gateway
curl -fsS http://127.0.0.1:10000/ready
curl -fsS http://127.0.0.1:10000/metrics
journalctl -u 'rsx-*' -f                  # live logs (JSON)
```

`deploy.sh --apply` already runs the full `/health` sweep and exits
non-zero if any endpoint stays down for 30 s.

## Rollback

Binaries are versioned by the build; to roll back, redeploy the prior
commit's binaries:

```
git checkout <prev-sha>
sudo RSX_DEPLOY_HOST="$(hostname -f)" ./deploy/deploy.sh --apply
```

To stop the cluster without uninstalling: `systemctl stop rsx.target`.
To take one process down: `systemctl stop rsx-me@btc`. State survives a
restart — ME replays its WAL, risk reloads from Postgres + WAL. A clean
`SIGTERM` (what systemd sends) is treated as a crash and recovers via
the one replay path (no special drain).

## Archive offload (do not skip)

The recorder writes the full ME stream to
`${PREFIX}/data/rsx/archive/<sid>/`. `RSX_RECORDER_RETAIN_DAYS=7`
prunes only the **local** rolling window; the permanent audit tier is
an object store. Run a daily offload (founder-owned cron), e.g.:

```
aws s3 sync ${PREFIX}/data/rsx/archive s3://rsx-archive/ --size-only
```

so every segment reaches S3/GCS well inside the 7-day local window
before pruning removes it. Keep the retention window comfortably longer
than the offload cadence. Do **not** lower retention to "save space" on
the root fs — put the archive on its own volume (prerequisite 3)
instead. Background: `bugs.md` RECORDER-ARCHIVE-DEV-DISK / FINDINGS #28.

## Files

- `systemd/` — unit per process (`rsx-me@`, `rsx-risk@`,
  `rsx-recorder@` are templated by instance) + `rsx.target`.
- `env/` — per-instance `EnvironmentFile`s (`${PREFIX}`/`${DATABASE_URL}`
  substituted at deploy time) and `secret.env.example`.
- `sysctl/99-rsx.conf` — casting/UDP buffer tuning.
- `deploy.sh` — idempotent installer (dry-run by default).

Spec: `specs/2/9-deploy.md`.
