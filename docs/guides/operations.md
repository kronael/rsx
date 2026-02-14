# RSX Recovery Runbook

Operational procedures for detecting and recovering from component failures in
the RSX perpetuals exchange. This runbook enables 24/7 operations with step-by-
step instructions for each failure scenario.

**Prerequisites:**
- Familiarity with [GUARANTEES.md](GUARANTEES.md) for data loss bounds and consistency guarantees
- Database credentials configured (set environment variables before operations):

```bash
export PGHOST=postgres-master
export PGUSER=rsx
export PGDATABASE=rsx
export PGPASSWORD=$(cat /srv/secrets/postgres_password)
# With these set, psql commands use: psql -c "QUERY"
# For clarity, this runbook shows full flags: psql -h $PGHOST -U $PGUSER -d $PGDATABASE -c "QUERY"
```

**Severity Classification:**
- **P0:** Data loss risk (seq gap, position mismatch, no main)
- **P1:** Trading halted or degraded (component down, backpressure)
- **P2:** Shadow component down (replica offline, mktdata down)

**Emergency Quick Reference:**

```bash
# Check all component health
curl http://me-btcusd:9100/health
curl http://risk-shard0:9200/health
curl http://gateway:8080/health
pg_isready -h postgres-master

# Check advisory locks (expect 1 per shard)
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT objid, COUNT(*) FROM pg_locks WHERE locktype='advisory' GROUP BY objid;"

# Quick position reconciliation (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- --shard-id 0

# Restart components (safe operations, idempotent)
systemctl restart rsx-matching@BTCUSD
systemctl restart rsx-risk@shard0
systemctl restart rsx-gateway
```

---

**Cross-References:**
- [GUARANTEES.md](GUARANTEES.md) - Data loss bounds and recovery time objectives
- [TESTING.md](specs/v1/TESTING.md) - Testing procedures referenced in verification steps
- Component-specific test specs in specs/v1/TESTING-*.md

---

## 1. Detection & Triage

### 1.1 Health Metrics Dashboard

**Critical Metrics (Monitor Every 10s):**

```
ME Heartbeat: Last seen < 5s ago
Risk Heartbeat: Last seen < 5s ago
Gateway Connections: > 0 active users
Postgres Connections: > 0 from Risk instances
ME WAL Flush Lag: p99 < 15ms
Risk Postgres Write Lag: p99 < 50ms
Risk Replica Lag: < 100ms
Advisory Lock Count: Exactly 1 per shard
```

**Alert Channels:**
- P0: PagerDuty + SMS + Slack #incidents
- P1: PagerDuty + Slack #ops
- P2: Slack #ops

### 1.2 Initial Triage Questions

When alert fires, answer these:

1. **Which component(s) down?** (check heartbeats)
2. **Are replicas healthy?** (check replica heartbeats)
3. **Is Postgres available?** (check pg_isready)
4. **Any disk full alerts?** (check df -h)
5. **Any network partition?** (check ping, traceroute)
6. **Recent deploys?** (check git log, last deploy time)

### 1.3 Triage Playbook

| Symptom | Likely Cause | Severity | Next Step |
|---------|--------------|----------|-----------|
| ME heartbeat timeout | ME crash or hang | P1 | Section 2.1 |
| Risk heartbeat timeout | Risk crash or hang | P1 | Section 2.2 |
| Gateway disconnect | Gateway crash or network | P1 | Section 2.3 |
| Postgres not accepting conn | PG crash or network | P0 | Section 2.4 |
| WAL flush lag spike | Disk slow or disk full | P1 | Section 3.1 |
| Postgres write lag spike | DB slow or lock contention | P1 | Section 3.2 |
| Replica lag > 500ms | Replica slow or network partition | P1 | Section 3.3 |
| Advisory lock count = 0 | No main (both crashed or network) | P0 | Section 4.1 |
| Advisory lock count = 2 | Split-brain (network partition) | P0 | Section 4.2 |
| Seq gap detected | Fill lost (critical bug) | P0 | Section 5.1 |
| Position mismatch | Fill not applied (critical bug) | P0 | Section 5.2 |

---

## 2. Single Component Recovery

### 2.1 Matching Engine Master Crash

**Detection:**
- ME heartbeat timeout (>5s)
- Risk cannot send orders (connection error)
- Users see order submission timeouts

**Pre-flight Checks:**
1. Is ME replica healthy? `systemctl status rsx-matching-replica@SYMBOL`
2. Is ME disk available? `df -h /srv/data/rsx/wal`
3. Is snapshot recent? `ls -lt /srv/data/rsx/snapshots`

**Recovery Steps:**

```bash
# 1. Check replica status
systemctl status rsx-matching-replica@BTCUSD

# 2. If replica healthy, promote it (fastest path)
#    Replica has same WAL, can serve immediately
#    Implementation: Update service discovery to point Risk to replica
#    Example for Consul:
consul kv put matching/BTCUSD/master "replica-host:9100"
#    Alternative: Update haproxy backend or DNS CNAME
#    Then restart Risk to reconnect:
systemctl restart rsx-risk@shard0

# 3. Restart crashed master (becomes new replica)
systemctl restart rsx-matching@BTCUSD

# 4. Verify master recovered
curl http://localhost:9100/health
# Expected: {"status": "ok", "role": "master", "seq": 12345678}

# 5. Check ME WAL has no gaps (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin dxs-verify -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --from-seq 0
# Expected: "No sequence gaps detected"

# 6. Verify Risk received all fills
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT symbol_id, last_seq, updated_at FROM tips WHERE symbol_id = 1;"
# Compare last_seq with ME health endpoint seq, should match within seconds

# 7. Run position reconciliation (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- \
  --symbol-id 1 \
  --max-delta 0
# Expected output: "All positions match fills. Checked: 1234 users"
```

**Expected Recovery Time:** 5-10s (replica promotion) OR 20-60s (master restart
from snapshot)

**Data Loss:** See [GUARANTEES.md](GUARANTEES.md) section 3.1 for ME crash bounds

**Rollback Plan:**
- If promoted replica has issues, roll back DNS to original master
- If original master cannot restart, cold start new instance from last snapshot

**Post-Recovery Verification:**
```bash
# Orders flow through
curl -X POST http://gateway:8080/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{"symbol":"BTCUSD","side":"buy","price":50000,"qty":0.1}'
# Expected: {"status":"ok","order_id":"01234567-89ab-cdef-0123-456789abcdef"}

# Check latest WAL file exists and is growing
ls -lth /srv/data/rsx/wal/1/ | head -n 3
stat /srv/data/rsx/wal/1/$(ls -t /srv/data/rsx/wal/1/ | head -1)
# Wait 1s and check size increased:
sleep 1 && stat /srv/data/rsx/wal/1/$(ls -t /srv/data/rsx/wal/1/ | head -1)

# Risk positions update
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT user_id, symbol_id, long_qty, short_qty, updated_at FROM positions WHERE symbol_id = 1 ORDER BY updated_at DESC LIMIT 5;"
```

---

### 2.2 Risk Engine Master Crash

**Detection:**
- Risk heartbeat timeout (>5s)
- Gateway cannot send orders (connection error)
- Users see order submission errors

**Pre-flight Checks:**
1. Is Risk replica healthy? `systemctl status rsx-risk-replica@shard0`
2. Is Postgres available? `pg_isready -h postgres-master`
3. Is ME still processing? `curl http://me-master:9100/health`

**Recovery Steps:**

```bash
# 1. Check replica status
systemctl status rsx-risk-replica@shard0

# 2. Replica should auto-promote (polls advisory lock every 500ms)
#    Wait 1s, then check if replica acquired lock
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT locktype, objid, pid, granted FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1 row with granted=t and pid from replica process

# 3. If replica did NOT auto-promote (bug or network issue):
#    Manually restart Risk instance
systemctl restart rsx-risk@shard0

# 4. Verify Risk recovered
curl http://localhost:9200/health
# Expected: {"status": "ok", "role": "main", "shard_id": 0}

# 5. Check Risk loaded positions from Postgres
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT COUNT(*) FROM positions WHERE user_id % 16 = 0;"
# Expected: Row count matching number of users in shard 0
# (Assuming 16 shards, shard 0 owns user_id % 16 = 0)

# 6. Check Risk requested DXS replay from ME
journalctl -u rsx-risk@shard0 -n 100 --no-pager | grep "requesting dxs replay"
# Expected to see line like: "requesting dxs replay from seq=12345678"

# 7. Wait for CaughtUp from all symbols (timeout after 60s)
timeout 60 journalctl -u rsx-risk@shard0 -f --no-pager | grep -m 1 "caught up"
# Expected to see: "caught up for all symbols, going live"

# 8. Verify positions match fills (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- \
  --shard-id 0 \
  --max-delta 0
# Expected output: "All positions match fills. Checked: 6250 users"
```

**Expected Recovery Time:** 2-5s (replica auto-promote) OR 10-30s (manual
restart + replay)

**Data Loss:** See [GUARANTEES.md](GUARANTEES.md) section 3.2 for Risk crash bounds

**Rollback Plan:**
- If replica promotion fails, restart original master
- If Postgres is slow, consider read-only mode (accept orders but don't persist)

**Post-Recovery Verification:**
```bash
# Orders accepted
curl -X POST http://gateway:8080/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{"symbol":"BTCUSD","side":"buy","price":50000,"qty":0.1}'
# Expected: {"status":"ok","order_id":"..."}

# Positions update
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT user_id, symbol_id, long_qty, short_qty, updated_at FROM positions WHERE user_id = 123 AND symbol_id = 1;"
# Expected: 1 row with recent updated_at timestamp

# Margin calculated
curl http://localhost:9200/api/v1/margin?user_id=123
# Expected: {"user_id":123,"available_margin":10000.00,"margin_ratio":0.15}
```

---

### 2.3 Gateway Crash

**Detection:**
- All user WebSocket connections drop
- Gateway heartbeat timeout
- Users see "Connection lost" errors

**Pre-flight Checks:**
1. Is Risk healthy? `curl http://risk:9200/health`
2. Is network available? `ping risk-master`

**Recovery Steps:**

```bash
# 1. Restart Gateway
systemctl restart rsx-gateway

# 2. Verify Gateway started
curl http://localhost:8080/health
# Expected: {"status": "ok", "connections": 0}

# 3. Users reconnect automatically (WebSocket client retry logic)
#    Monitor connection count
watch 'curl -s http://localhost:8080/health | jq .connections'

# 4. Users resubmit pending orders (client responsibility)
#    Gateway deduplicates via UUIDv7 order_id (5min window)
```

**Expected Recovery Time:** <1s (stateless restart)

**Data Loss:** None (Gateway is stateless, users resubmit in-flight orders)

**Rollback Plan:** N/A (stateless, no rollback needed)

**Post-Recovery Verification:**
```bash
# Users connected
curl http://localhost:8080/health | jq .connections
# Expected: >0

# Orders routed to Risk
tail -f /var/log/rsx/gateway.log | grep "Order routed"
```

---

### 2.4 Postgres Master Crash

**Detection:**
- Postgres not accepting connections
- Risk write-behind stalls
- `pg_isready` fails

**Pre-flight Checks:**
1. Is Postgres replica available? `pg_isready -h postgres-replica`
2. Is Risk still processing? `curl http://risk:9200/health`
3. Is disk full? `df -h /var/lib/postgresql`

**Recovery Steps:**

```bash
# 1. Check Postgres status
systemctl status postgresql

# 2. If Postgres crashed, restart
systemctl restart postgresql

# 3. If Postgres won't restart (corruption?), promote replica
#    (Requires synchronous replication for 0ms loss)
#    This is Postgres-specific, example for streaming replication:

# On replica:
pg_ctl promote -D /var/lib/postgresql/14/main

# Update DNS or connection string to point to new master
# (Implementation depends on infrastructure)

# 4. Verify Postgres is accepting connections
pg_isready -h postgres-master
# Expected: accepting connections

# 5. Risk will auto-reconnect and resume flushing
#    Check Risk logs for reconnection
journalctl -u rsx-risk@shard0 -n 50 --no-pager | grep -i "postgres"
# Expected to see: "connected to postgres" or "reconnected"

# 6. Verify positions are being written
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT MAX(updated_at) FROM positions;"
# Expected: timestamp within last few seconds (e.g., "2026-02-08 14:32:15.123")

# 7. Verify no data loss (if synchronous replica)
#    All committed transactions should be present
psql -h postgres-master -U rsx -d rsx -c "SELECT COUNT(*) FROM positions;"
# Compare with pre-crash count (should be equal or higher)
# Save pre-crash count first: echo "12345" > /tmp/positions_count_before.txt
```

**Expected Recovery Time:** 30-60s (Postgres recovery process)

**Data Loss:** See [GUARANTEES.md](GUARANTEES.md) section 3.3 for Postgres crash bounds

**Rollback Plan:**
- If promoted replica has issues, restore from backup + replay ME WAL
- If Postgres corruption detected, restore from backup + replay from tip

**Post-Recovery Verification:**
```bash
# Risk writing to Postgres (watch for 10s)
watch -n 1 'psql -h postgres-master -U rsx -d rsx -c "SELECT MAX(updated_at) FROM positions;"'
# Press Ctrl-C after confirming timestamp advances every second

# Write-behind lag normal
curl http://risk:9200/metrics | grep write_behind_lag_ms
# Expected: risk_postgres_write_lag_ms_p99 < 10
```

---

## 3. Performance Degradation

### 3.1 WAL Flush Lag Spike (ME)

**Detection:**
- `me_wal_flush_lag_ms` p99 > 15ms
- Backpressure alerts (ME stalling on order processing)

**Likely Causes:**
- Disk slow (check `iostat -x 1`)
- Disk full (check `df -h`)
- Competing I/O (check `iotop`)

**Immediate Actions:**

```bash
# 1. Check disk usage
df -h /srv/data/rsx/wal
# If >90% full, rotate and archive old WAL files

# 2. Check disk I/O (10 samples, 1 second apart)
iostat -x 1 10
# Look for high await times (>10ms) in output columns
# Focus on device with /srv/data/rsx/wal mounted

# 3. Check if fsync is slow (run for 10s, then Ctrl-C)
mkdir -p /home/onvos/sandbox/rsx/tmp
timeout 10 strace -p $(pgrep rsx-matching) -e fsync -T 2>&1 | tee /home/onvos/sandbox/rsx/tmp/fsync_trace.log
# Review tmp/fsync_trace.log, look for: fsync(3) = 0 <0.015234>
# If times consistently >0.010 seconds, disk is slow

# 4. If disk full, archive old WAL files
find /srv/data/rsx/wal -name "*.wal" -mmin +20 -exec gzip {} \;
mkdir -p /srv/archive/wal
mv /srv/data/rsx/wal/*.wal.gz /srv/archive/wal/

# 5. If disk slow, check for hardware issues
smartctl -a /dev/sda
# Look for reallocated sectors, pending sectors

# 6. If competing I/O, identify and throttle
iotop -o
# Kill or nice competing processes
```

**Mitigation:**
- Increase WAL buffer size (reduces flush frequency)
- Add more disk bandwidth (faster SSD, RAID)
- Offload WAL archival to separate disk

**Post-Mitigation Verification:**
```bash
# Flush lag back to normal
curl http://me:9100/metrics | grep wal_flush_lag_ms
# Expected: p99 < 10ms

# No backpressure stalls
curl http://me:9100/metrics | grep backpressure_stalls
# Expected: 0/sec
```

---

### 3.2 Postgres Write Lag Spike (Risk)

**Detection:**
- `risk_postgres_write_lag_ms` p99 > 50ms
- Backpressure alerts (Risk stalling on fill processing)

**Likely Causes:**
- DB lock contention (check `pg_locks`)
- DB vacuum/autovacuum running (check `pg_stat_activity`)
- DB slow queries (check `pg_stat_statements`)
- Network latency to DB (check `ping postgres-master`)

**Immediate Actions:**

```bash
# 1. Check active queries
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT pid, usename, query, state, wait_event, query_start FROM pg_stat_activity WHERE state = 'active';"

# 2. Check locks
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT locktype, relation::regclass, mode, pid, granted FROM pg_locks WHERE NOT granted;"

# 3. Check if vacuum running
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT pid, datname, relid::regclass, phase, heap_blks_scanned, heap_blks_total FROM pg_stat_progress_vacuum;"

# 4. If vacuum running and causing contention, consider canceling
#    (Only if P1 severity and trading is degraded)
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE query ILIKE '%VACUUM%' AND state = 'active';"

# 5. Check for slow queries (requires pg_stat_statements extension)
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT LEFT(query, 80) AS query, ROUND(mean_exec_time::numeric, 2) AS mean_ms, calls FROM pg_stat_statements ORDER BY mean_exec_time DESC LIMIT 10;"

# 6. If network latency, check route
ping -c 10 postgres-master
traceroute postgres-master

# 7. If DB overloaded, scale up (add read replicas, increase resources)
#    (Long-term fix, not immediate)
```

**Mitigation:**
- Tune autovacuum to run during off-peak hours
- Increase Postgres shared_buffers, work_mem
- Add connection pooling (PgBouncer) to reduce connection overhead
- Partition large tables (fills, funding_payments) by date

**Post-Mitigation Verification:**
```bash
# Write lag back to normal
curl http://risk:9200/metrics | grep postgres_write_lag_ms
# Expected: p99 < 10ms

# No backpressure stalls
curl http://risk:9200/metrics | grep backpressure_stalls
# Expected: 0/sec
```

---

### 3.3 Replica Lag Spike

**Detection:**
- `risk_replica_lag_ms` > 500ms
- Warning alert (potential 100ms loss if dual crash)

**Likely Causes:**
- Replica processing slow (CPU bound, check `top`)
- Network partition (check `ping replica`)
- Replica disk slow (check `iostat`)

**Immediate Actions:**

```bash
# 1. Check replica health
systemctl status rsx-risk-replica@shard0

# 2. Check replica lag
curl http://replica:9200/metrics | grep replica_lag_ms

# 3. Check network to replica
ping -c 10 replica-host
# If packet loss or high latency, investigate network

# 4. Check replica CPU usage
ssh replica-host top -b -n 1 | grep rsx-risk

# 5. If replica overloaded, check if processing all symbols
#    (Replica should only buffer fills, not process margin)
journalctl -u rsx-risk-replica@shard0 -n 100 | grep "Processing"

# 6. If replica lagging behind main tip sync, check SPSC ring
#    (Should see log: "Tip sync ring full, main stalling")
journalctl -u rsx-risk@shard0 -n 100 | grep "Tip sync"
```

**Mitigation:**
- If replica cannot keep up, consider accepting higher lag (update threshold)
- If network partition, wait for partition to heal (replica will catch up)
- If replica hardware insufficient, scale up replica resources

**Post-Mitigation Verification:**
```bash
# Replica lag back to normal
curl http://replica:9200/metrics | grep replica_lag_ms
# Expected: <100ms

# Replica caught up with main
curl http://main:9200/metrics | grep tip_seq
curl http://replica:9200/metrics | grep tip_seq
# Compare, should be within 100ms lag
```

---

## 4. Critical Failures (P0)

### 4.1 No Main (Advisory Lock Count = 0)

**Detection:**
- `advisory_lock_count` = 0 for shard
- Both master and replica crashed or network partition

**Severity:** P0 (trading halted, no instance processing orders)

**Immediate Actions:**

```bash
# 1. Check both master and replica status
systemctl status rsx-risk@shard0
systemctl status rsx-risk-replica@shard0

# 2. If both crashed, restart master first
systemctl restart rsx-risk@shard0

# 3. Wait for master to acquire lock
sleep 5
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT locktype, objid, pid, granted FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1 row with granted=t and pid from master process

# 4. Restart replica
systemctl restart rsx-risk-replica@shard0

# 5. Verify replica did NOT acquire lock (main holds it)
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: count = 1 (only main)

# 6. Run position reconciliation (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- \
  --shard-id 0 \
  --max-delta 0
# Expected: "All positions match fills"
```

**Expected Recovery Time:** 30-60s (both instances restart + replay)

**Data Loss:** See [GUARANTEES.md](GUARANTEES.md) section 4.1 for dual crash bounds

**Post-Recovery Verification:**
```bash
# Main acquired lock
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT locktype, objid, pid, granted FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1 row with granted=t

# Orders accepted
curl -X POST http://gateway:8080/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{"symbol":"BTCUSD","side":"buy","price":50000,"qty":0.1}'
# Expected: {"status":"ok","order_id":"..."}

# Positions consistent (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- --shard-id 0
# Expected: "All positions match fills"
```

---

### 4.2 Split-Brain (Advisory Lock Count = 2)

**Detection:**
- `advisory_lock_count` = 2 for shard
- Both master and replica think they are main

**Severity:** P0 (data corruption risk, dual writes to Postgres)

**Immediate Actions:**

```bash
# 1. HALT both instances immediately
systemctl stop rsx-risk@shard0
systemctl stop rsx-risk-replica@shard0

# 2. Check Postgres locks (should be 0 after halt)
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT locktype, objid, pid FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 0 rows (both stopped)

# 3. Identify which instance has newer state
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT instance_id, symbol_id, last_seq, updated_at FROM tips WHERE instance_id IN (0, 100) ORDER BY symbol_id, updated_at DESC;"
# (Assuming master=0, replica=100)
# Choose instance with higher last_seq values

# 4. Choose the instance with higher tips (more recent state)
#    Example: If master (instance_id=0) has higher tips, keep master state

# 5. Restart master only
systemctl start rsx-risk@shard0

# 6. Wait for master to acquire lock
sleep 5
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: count = 1

# 7. Restart replica (will NOT acquire lock, main holds it)
systemctl start rsx-risk-replica@shard0

# 8. Verify only 1 lock held
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: count = 1

# 9. Run position reconciliation (critical!)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- \
  --shard-id 0 \
  --max-delta 0
# If ANY mismatch detected, investigate which fills were applied by which instance
```

**Expected Recovery Time:** 30-60s (manual intervention required)

**Data Loss:** Potentially unbounded (split-brain requires manual reconciliation)

**Post-Recovery Verification:**
```bash
# Only 1 lock held
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: count = 1

# Positions consistent
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- --shard-id 0
# Expected: "All positions match fills"

# No duplicate fills applied
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT symbol_id, seq, COUNT(*) FROM fills WHERE symbol_id = 1 GROUP BY symbol_id, seq HAVING COUNT(*) > 1;"
# Expected: 0 rows (no duplicates)
```

**Root Cause Analysis:**
- Network partition between Risk instances and Postgres?
- Advisory lock timeout not configured correctly?
- Bug in lease renewal logic?

---

### 5.1 Seq Gap Detected

**Detection:**
- ME WAL has missing seq numbers (gap in sequence)
- Position reconciliation fails (missing fills)

**Severity:** P0 (fill lost, critical bug)

**Immediate Actions:**

```bash
# 1. HALT all trading immediately
systemctl stop rsx-gateway

# 2. Identify gap location (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin dxs-verify -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --from-seq 0
# Expected output: "Gap detected at seq=12345678" or "No gaps detected"

# 3. Check if gap in WAL file or just in-memory
ls -lt /srv/data/rsx/wal/1/
# Find file containing seq before gap and seq after gap

# 4. Inspect WAL file manually (view hex dump of first 512 bytes)
xxd -l 512 /srv/data/rsx/wal/1/$(ls -t /srv/data/rsx/wal/1/ | head -1)
# Look for record structure: 16B header + payload
# Seq field is in header at offset 8 (i64 little-endian)

# 5. Check if DXS replay server can serve missing seq
curl -X POST http://me:9100/dxs/replay \
  -H "Content-Type: application/json" \
  -d '{"stream_id":1,"from_seq":12345678}' | head -c 200
# If server returns binary data (starts with fill record), gap is in Risk only, not ME
# If server returns error or empty, gap is in ME WAL (critical)

# 6. If gap in ME WAL (fill actually lost):
#    This is a CRITICAL BUG, need immediate patch
#    Possible causes:
#    - WAL writer bug (didn't write fill)
#    - Disk corruption (fill written but corrupted)
#    - fsync lied (fill flushed to cache, not disk)

# 7. Investigate logs for fill emission
journalctl -u rsx-matching@BTCUSD -S "10 minutes ago" | grep "seq=12345678"

# 8. If fill emitted but not in WAL: WAL writer bug
# 9. If fill not emitted: matching engine bug
# 10. If fill in WAL but corrupted: disk corruption

# 11. Notify engineering team for emergency patch
```

**Mitigation:**
- If gap is small (1-2 fills): manually reconstruct from logs, apply to Risk
- If gap is large: restore from last known good state, replay from backup

**Post-Mitigation Verification:**
```bash
# No gaps in WAL (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin dxs-verify -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --from-seq 0
# Expected: "No sequence gaps detected"

# Positions match fills (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- --symbol-id 1
# Expected: "All positions match fills"
```

---

### 5.2 Position Mismatch

**Detection:**
- Position reconciliation query fails
- `position.long_qty != sum(fills where side=buy)`

**Severity:** P0 (position drift, margin calculation incorrect)

**Immediate Actions:**

```bash
# 1. HALT all trading immediately
systemctl stop rsx-gateway

# 2. Run detailed reconciliation (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- \
  --shard-id 0 \
  --verbose
# Expected output: List of mismatched positions with deltas

# 3. For each mismatched position, compare with fills
psql -h postgres-master -U rsx -d rsx -c "
SELECT user_id, symbol_id,
       (SELECT COALESCE(SUM(qty), 0) FROM fills WHERE taker_user_id = p.user_id AND symbol_id = p.symbol_id AND side = 0) AS fills_buy,
       (SELECT COALESCE(SUM(qty), 0) FROM fills WHERE taker_user_id = p.user_id AND symbol_id = p.symbol_id AND side = 1) AS fills_sell,
       long_qty, short_qty
FROM positions p
WHERE user_id = 123 AND symbol_id = 1;
"
# Compare fills_buy with long_qty, fills_sell with short_qty

# 4. Check if fills are missing in Risk (not applied)
psql -h postgres-master -U rsx -d rsx -c "
SELECT seq FROM fills
WHERE symbol_id = 1 AND taker_user_id = 123
AND seq > (SELECT last_seq FROM tips WHERE symbol_id = 1 AND instance_id = 0);
"
# If rows returned, Risk didn't process these fills yet

# 5. Check if fills were double-applied (dedup failed)
#    (Check Risk logs for duplicate seq)
journalctl -u rsx-risk@shard0 -S "1 hour ago" | grep "Duplicate fill"

# 6. Possible causes:
#    - Fill not applied (Risk bug or crash during apply)
#    - Fill double-applied (dedup bug)
#    - Fill applied to wrong user (routing bug)

# 7. Correct positions manually (emergency fix)
#    Recompute from fills and update Postgres
psql -h postgres-master -U rsx -d rsx -c "
UPDATE positions SET
  long_qty = (SELECT COALESCE(SUM(qty), 0) FROM fills WHERE taker_user_id = 123 AND symbol_id = 1 AND side = 0),
  short_qty = (SELECT COALESCE(SUM(qty), 0) FROM fills WHERE taker_user_id = 123 AND symbol_id = 1 AND side = 1),
  updated_at = NOW()
WHERE user_id = 123 AND symbol_id = 1;
"
# Verify update:
psql -h postgres-master -U rsx -d rsx -c \
  "SELECT user_id, symbol_id, long_qty, short_qty FROM positions WHERE user_id = 123 AND symbol_id = 1;"

# 8. Notify engineering team for root cause analysis
```

**Mitigation:**
- Rebuild all positions from fills (time-consuming but correct)
- Identify and fix bug in Risk fill application logic

**Post-Mitigation Verification:**
```bash
# All positions match fills (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin reconcile-positions -- --shard-id 0
# Expected: "All positions match fills. Checked: 6250 users"

# Margin recalculated correctly
curl http://risk:9200/api/v1/margin?user_id=123
# Expected: {"user_id":123,"available_margin":10000.00,"margin_ratio":0.15}
```

---

## 6. Chaos Testing Scenarios (Proactive)

### 6.1 Random Component Kill

**Purpose:** Verify system survives random crashes without data loss

**Procedure:**

```bash
# Run chaos test for 10min (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin chaos-test -- \
  --duration 10m \
  --kill-interval 30s \
  --components ME,Risk,Gateway

# Chaos test will:
# - Randomly kill component every 30s
# - Verify recovery within RTO
# - Verify no data loss (position = fills)
# - Verify no seq gaps
# - Verify advisory lock always held

# Expected output: "System survived 10min chaos test. All invariants hold."
```

---

### 6.2 Correlated Failures

**Purpose:** Verify bounded loss on dual component crash

**Procedure:**

```bash
# Crash ME master + replica within 10ms (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin chaos-test -- \
  --scenario dual-me-crash \
  --symbol BTCUSD

# Verify:
# - Data loss <= 10ms orders
# - Fills not lost (WAL available)
# - Recovery time <= 20s
# - Positions consistent after recovery

# Crash Risk master + replica within 10ms (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin chaos-test -- \
  --scenario dual-risk-crash \
  --shard-id 0

# Verify:
# - Data loss <= 100ms positions
# - Positions reconstructed from ME fills
# - Recovery time <= 60s
# - Advisory lock reacquired
```

---

### 6.3 Disk Failure Simulation

**Purpose:** Verify backpressure and graceful degradation

**Procedure:**

```bash
# Simulate disk full (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin chaos-test -- \
  --scenario disk-full \
  --component ME

# Verify:
# - ME stalls on WAL buffer full
# - No silent data loss
# - Alert fires (disk full)
# - Recovery after disk space freed

# Simulate disk slow (inject latency, from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin chaos-test -- \
  --scenario disk-slow \
  --component ME \
  --latency 50ms

# Verify:
# - WAL flush lag increases
# - Backpressure enforced when lag > 10ms
# - Alert fires (flush lag)
```

---

### 6.4 Network Partition Simulation

**Purpose:** Verify partition tolerance and replay

**Procedure:**

```bash
# Partition Risk <-> ME for 5min (from project root)
cd /home/onvos/sandbox/rsx && cargo run --bin chaos-test -- \
  --scenario partition \
  --source Risk \
  --target ME \
  --duration 5m

# Verify:
# - ME continues matching (buffers fills in WAL)
# - Risk detects partition (heartbeat timeout)
# - Partition heals
# - Risk replays from ME WAL (5min of fills)
# - Positions consistent after replay
# - No data loss (all fills replayed)
```

---

## 7. Monitoring Queries

### 7.1 Advisory Lock Status

```bash
# Check advisory locks per shard
psql -h postgres-master -U rsx -d rsx -c "
SELECT objid AS shard_id, COUNT(*) AS lock_count, array_agg(pid) AS holder_pids
FROM pg_locks
WHERE locktype = 'advisory'
GROUP BY objid
ORDER BY objid;
"
# Expected: Each shard (0-15) has exactly 1 lock
# Example output:
#  shard_id | lock_count | holder_pids
# ----------+------------+-------------
#         0 |          1 | {12345}
#         1 |          1 | {12346}
```

### 7.2 Position Reconciliation

```bash
# Quick reconciliation check (sample 1%)
psql -h postgres-master -U rsx -d rsx -c "
WITH fills_sum AS (
  SELECT
    taker_user_id AS user_id,
    symbol_id,
    SUM(CASE WHEN side = 0 THEN qty ELSE 0 END) AS fills_buy,
    SUM(CASE WHEN side = 1 THEN qty ELSE 0 END) AS fills_sell
  FROM fills
  WHERE taker_user_id % 100 = 0  -- Sample 1%
  GROUP BY taker_user_id, symbol_id
)
SELECT
  p.user_id,
  p.symbol_id,
  p.long_qty,
  f.fills_buy,
  p.short_qty,
  f.fills_sell,
  CASE
    WHEN p.long_qty != f.fills_buy OR p.short_qty != f.fills_sell THEN 'MISMATCH'
    ELSE 'OK'
  END AS status
FROM positions p
JOIN fills_sum f ON p.user_id = f.user_id AND p.symbol_id = f.symbol_id
WHERE p.long_qty != f.fills_buy OR p.short_qty != f.fills_sell;
"
# Expected: 0 rows (all match)
# If rows returned, positions are inconsistent - escalate to P0
```

### 7.3 Tips Monotonic Check

```sql
-- Verify tips never decreased
WITH tips_with_lag AS (
  SELECT
    instance_id,
    symbol_id,
    last_seq,
    updated_at,
    LAG(last_seq) OVER (PARTITION BY instance_id, symbol_id ORDER BY updated_at) AS prev_seq
  FROM tips
)
SELECT instance_id, symbol_id, last_seq, prev_seq, updated_at
FROM tips_with_lag
WHERE prev_seq IS NOT NULL AND last_seq < prev_seq;

-- Expected: 0 rows (tips always increase)
```

### 7.4 Funding Zero-Sum Check

```bash
# Verify funding payments sum to zero per interval
psql -h postgres-master -U rsx -d rsx -c "
SELECT
  symbol_id,
  settlement_ts,
  SUM(amount) AS total_funding
FROM funding_payments
GROUP BY symbol_id, settlement_ts
HAVING ABS(SUM(amount)) > 100;
"
# Expected: 0 rows (all zero-sum within rounding error of 100 units)
# If rows returned, funding calculation has bug - escalate to engineering
```

---

## 8. Escalation Contacts

| Severity | Contact | Response Time |
|----------|---------|---------------|
| P0 (data loss risk) | Engineering Lead + CTO | 5min |
| P1 (trading halted) | Engineering Oncall | 15min |
| P2 (shadow component) | Engineering Team (Slack) | 1hr |

**Escalation Path:**
1. P2 alert → Slack #ops
2. If no response in 30min → page Engineering Oncall
3. P1 alert → page Engineering Oncall
4. If no response in 15min → page Engineering Lead
5. P0 alert → page Engineering Lead + CTO immediately

---

## 9. Post-Incident Checklist

After any P0/P1 incident, complete this checklist:

- [ ] Root cause identified
- [ ] Data loss quantified (if any)
- [ ] All invariants verified (position=fills, tips monotonic, etc.)
- [ ] Monitoring dashboards reviewed (any missed alerts?)
- [ ] Runbook updated (new scenario or procedure)
- [ ] Postmortem scheduled (within 48hr)
- [ ] Code fix deployed (if bug identified)
- [ ] Chaos test added (to prevent regression)

---

This runbook is a living document. Update after every incident with lessons
learned and improved procedures.
