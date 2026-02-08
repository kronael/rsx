# RSX Recovery Runbook

Operational procedures for detecting and recovering from component failures in
the RSX perpetuals exchange. This runbook enables 24/7 operations with step-by-
step instructions for each failure scenario.

**Prerequisites:** Familiarity with [GUARANTEES.md](GUARANTEES.md) for data
loss bounds and consistency guarantees.

**Severity Classification:**
- **P0:** Data loss risk (seq gap, position mismatch, no main)
- **P1:** Trading halted or degraded (component down, backpressure)
- **P2:** Shadow component down (replica offline, mktdata down)

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
#    Update DNS or load balancer to point to replica
#    (Actual command depends on infrastructure)
# For now, assume manual process:
echo "Promote replica to master:"
echo "  - Update Consul KV: matching/BTCUSD/master = replica_addr"
echo "  - Restart Risk to reconnect to new master"

# 3. Restart crashed master (becomes new replica)
systemctl restart rsx-matching@BTCUSD

# 4. Verify master recovered
curl http://localhost:9100/health
# Expected: {"status": "ok", "role": "master", "seq": 12345678}

# 5. Check ME WAL has no gaps
cargo run --bin rsx-wal-check -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --from-seq 0

# 6. Verify Risk received all fills
psql -c "SELECT symbol_id, last_seq FROM tips WHERE symbol_id = 1;"
# Compare with ME seq, should be within 10ms lag

# 7. Run position reconciliation
cargo run --bin rsx-position-reconcile -- \
  --symbol-id 1 \
  --max-delta 0

# Expected: "All positions match fills."
```

**Expected Recovery Time:** 5-10s (replica promotion) OR 20-60s (master restart
from snapshot)

**Data Loss:** 10ms orders (accepted but not in WAL), 0ms fills

**Rollback Plan:**
- If promoted replica has issues, roll back DNS to original master
- If original master cannot restart, cold start new instance from last snapshot

**Post-Recovery Verification:**
```bash
# Orders flow through
curl -X POST http://gateway/api/v1/orders \
  -d '{"symbol":"BTCUSD","side":"buy","price":50000,"qty":0.1}'

# Fills emit correctly
tail -f /srv/data/rsx/wal/1/1_*.wal | xxd | head

# Risk positions update
psql -c "SELECT * FROM positions WHERE symbol_id = 1 LIMIT 5;"
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
psql -c "SELECT * FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1 row with pid from replica process

# 3. If replica did NOT auto-promote (bug or network issue):
#    Manually restart Risk instance
systemctl restart rsx-risk@shard0

# 4. Verify Risk recovered
curl http://localhost:9200/health
# Expected: {"status": "ok", "role": "main", "shard_id": 0}

# 5. Check Risk loaded positions from Postgres
psql -c "SELECT COUNT(*) FROM positions WHERE user_id % 16 = 0;"
# (Assuming 16 shards, shard 0 owns user_id % 16 = 0)

# 6. Check Risk requested DXS replay from ME
#    (Should see log: "Requesting DXS replay from seq=12345678")
journalctl -u rsx-risk@shard0 -n 100 | grep "DXS replay"

# 7. Wait for CaughtUp from all symbols
#    (Should see log: "CaughtUp for all symbols, going live")
journalctl -u rsx-risk@shard0 -f | grep "CaughtUp"

# 8. Verify positions match fills
cargo run --bin rsx-position-reconcile -- \
  --shard-id 0 \
  --max-delta 0

# Expected: "All positions match fills."
```

**Expected Recovery Time:** 2-5s (replica auto-promote) OR 10-30s (manual
restart + replay)

**Data Loss:** 10ms positions (fills received but not flushed to Postgres)

**Rollback Plan:**
- If replica promotion fails, restart original master
- If Postgres is slow, consider read-only mode (accept orders but don't persist)

**Post-Recovery Verification:**
```bash
# Orders accepted
curl -X POST http://gateway/api/v1/orders \
  -d '{"symbol":"BTCUSD","side":"buy","price":50000,"qty":0.1}'

# Positions update
psql -c "SELECT * FROM positions WHERE user_id = 123 AND symbol_id = 1;"

# Margin calculated
curl http://localhost:9200/api/v1/margin?user_id=123
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

**Data Loss:** 0ms for critical state (Gateway is stateless), orders in flight
lost (users resubmit)

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
#    Monitor write-behind lag
psql -c "SELECT pg_stat_get_db_xact_commit(oid) FROM pg_database WHERE datname = 'rsx';"

# 6. Verify positions are being written
psql -c "SELECT MAX(updated_at) FROM positions;"
# Expected: recent timestamp (within 10ms)

# 7. Verify no data loss (if synchronous replica)
#    All committed transactions should be present
psql -c "SELECT COUNT(*) FROM positions;"
# Compare with pre-crash count (should be equal or higher)
```

**Expected Recovery Time:** 30-60s (Postgres recovery process)

**Data Loss:** 0ms for committed transactions, 10ms for uncommitted batches
(Risk replays from ME fills)

**Rollback Plan:**
- If promoted replica has issues, restore from backup + replay ME WAL
- If Postgres corruption detected, restore from backup + replay from tip

**Post-Recovery Verification:**
```bash
# Risk writing to Postgres
watch 'psql -c "SELECT MAX(updated_at) FROM positions;"'

# Write-behind lag < 10ms
curl http://risk:9200/metrics | grep write_behind_lag_ms
# Expected: p99 < 10ms
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

# 2. Check disk I/O
iostat -x 1 10
# Look for high await times (>10ms)

# 3. Check if fsync is slow
strace -p $(pgrep rsx-matching) -e fsync -T
# Look for fsync taking >10ms

# 4. If disk full, archive old WAL files
find /srv/data/rsx/wal -name "*.wal" -mmin +20 -exec gzip {} \;
mv /srv/data/rsx/wal/*.wal.gz /srv/archive/

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
psql -c "SELECT pid, query, state, wait_event FROM pg_stat_activity WHERE state = 'active';"

# 2. Check locks
psql -c "SELECT * FROM pg_locks WHERE NOT granted;"

# 3. Check if vacuum running
psql -c "SELECT * FROM pg_stat_progress_vacuum;"

# 4. If vacuum running and causing contention, consider canceling
#    (Only if P1 severity and trading is degraded)
psql -c "SELECT pg_cancel_backend(pid) FROM pg_stat_activity WHERE query LIKE '%VACUUM%';"

# 5. Check for slow queries
psql -c "SELECT query, mean_exec_time, calls FROM pg_stat_statements ORDER BY mean_exec_time DESC LIMIT 10;"

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
psql -c "SELECT * FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1 row with pid from master process

# 4. Restart replica
systemctl restart rsx-risk-replica@shard0

# 5. Verify replica did NOT acquire lock (main holds it)
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1 (only main)

# 6. Run position reconciliation
cargo run --bin rsx-position-reconcile -- \
  --shard-id 0 \
  --max-delta 0
```

**Expected Recovery Time:** 30-60s (both instances restart + replay)

**Data Loss:** 100ms positions (both crashed before flush, worst case)

**Post-Recovery Verification:**
```bash
# Main acquired lock
psql -c "SELECT * FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"

# Orders accepted
curl -X POST http://gateway/api/v1/orders \
  -d '{"symbol":"BTCUSD","side":"buy","price":50000,"qty":0.1}'

# Positions consistent
cargo run --bin rsx-position-reconcile -- --shard-id 0
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
psql -c "SELECT * FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 0 rows

# 3. Identify which instance has newer state
psql -c "SELECT instance_id, symbol_id, last_seq, updated_at FROM tips WHERE instance_id IN (0, 100) ORDER BY symbol_id, updated_at DESC;"
# (Assuming master=0, replica=100)

# 4. Choose the instance with higher tips (more recent state)
#    Let's say master has higher tips, keep master state

# 5. Restart master only
systemctl start rsx-risk@shard0

# 6. Wait for master to acquire lock
sleep 5
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1

# 7. Restart replica (will NOT acquire lock, main holds it)
systemctl start rsx-risk-replica@shard0

# 8. Verify only 1 lock held
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1

# 9. Run position reconciliation (critical!)
cargo run --bin rsx-position-reconcile -- \
  --shard-id 0 \
  --max-delta 0
# If ANY mismatch, investigate which fills were applied by which instance
```

**Expected Recovery Time:** 30-60s (manual intervention required)

**Data Loss:** Potentially unbounded (if both instances wrote to Postgres,
manual merge required)

**Post-Recovery Verification:**
```bash
# Only 1 lock held
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1

# Positions consistent
cargo run --bin rsx-position-reconcile -- --shard-id 0

# No duplicate fills applied
psql -c "SELECT symbol_id, seq, COUNT(*) FROM fills WHERE symbol_id = 1 GROUP BY symbol_id, seq HAVING COUNT(*) > 1;"
# Expected: 0 rows
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

# 2. Identify gap location
cargo run --bin rsx-wal-check -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --from-seq 0
# Expected output: "Gap detected: seq 12345678 missing"

# 3. Check if gap in WAL file or just in-memory
ls -lt /srv/data/rsx/wal/1/
# Find file containing seq before gap and seq after gap

# 4. Inspect WAL files manually
xxd /srv/data/rsx/wal/1/1_12345000_12346000.wal | grep -A 5 -B 5 "seq"

# 5. Check if DXS replay server can serve missing seq
curl -X POST http://me:9100/dxs/replay \
  -d '{"stream_id":1,"from_seq":12345678}'
# If server returns record, gap is in Risk, not ME

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
# No gaps in WAL
cargo run --bin rsx-wal-check -- --wal-dir /srv/data/rsx/wal/1 --from-seq 0
# Expected: "No gaps detected"

# Positions match fills
cargo run --bin rsx-position-reconcile -- --symbol-id 1
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

# 2. Run detailed reconciliation
cargo run --bin rsx-position-reconcile -- \
  --shard-id 0 \
  --verbose
# Expected output: List of mismatched positions

# 3. For each mismatched position, compare with fills
psql -c "
SELECT user_id, symbol_id,
       (SELECT SUM(qty) FROM fills WHERE taker_user_id = p.user_id AND symbol_id = p.symbol_id AND side = 0) AS fills_buy,
       (SELECT SUM(qty) FROM fills WHERE taker_user_id = p.user_id AND symbol_id = p.symbol_id AND side = 1) AS fills_sell,
       long_qty, short_qty
FROM positions p
WHERE user_id = 123 AND symbol_id = 1;
"

# 4. Check if fills are missing in Risk (not applied)
psql -c "
SELECT seq FROM fills
WHERE symbol_id = 1 AND taker_user_id = 123
AND seq > (SELECT last_seq FROM tips WHERE symbol_id = 1);
"
# If rows returned, Risk didn't process these fills

# 5. Check if fills were double-applied (dedup failed)
#    (Check Risk logs for duplicate seq)
journalctl -u rsx-risk@shard0 -S "1 hour ago" | grep "Duplicate fill"

# 6. Possible causes:
#    - Fill not applied (Risk bug or crash during apply)
#    - Fill double-applied (dedup bug)
#    - Fill applied to wrong user (routing bug)

# 7. Correct positions manually (emergency fix)
#    Recompute from fills and update Postgres
psql -c "
UPDATE positions SET
  long_qty = (SELECT SUM(qty) FROM fills WHERE taker_user_id = 123 AND symbol_id = 1 AND side = 0),
  short_qty = (SELECT SUM(qty) FROM fills WHERE taker_user_id = 123 AND symbol_id = 1 AND side = 1)
WHERE user_id = 123 AND symbol_id = 1;
"

# 8. Notify engineering team for root cause analysis
```

**Mitigation:**
- Rebuild all positions from fills (time-consuming but correct)
- Identify and fix bug in Risk fill application logic

**Post-Mitigation Verification:**
```bash
# All positions match fills
cargo run --bin rsx-position-reconcile -- --shard-id 0
# Expected: "All positions match fills"

# Margin recalculated correctly
curl http://risk:9200/api/v1/margin?user_id=123
```

---

## 6. Chaos Testing Scenarios (Proactive)

### 6.1 Random Component Kill

**Purpose:** Verify system survives random crashes without data loss

**Procedure:**

```bash
# Run chaos test for 10min
cargo run --bin rsx-chaos-test -- \
  --duration 10m \
  --kill-interval 30s \
  --components ME,Risk,Gateway

# Chaos test will:
# - Random kill component every 30s
# - Verify recovery within RTO
# - Verify no data loss (position = fills)
# - Verify no seq gaps
# - Verify advisory lock always held

# Expected: System survives, all invariants hold
```

---

### 6.2 Correlated Failures

**Purpose:** Verify bounded loss on dual component crash

**Procedure:**

```bash
# Crash ME master + replica within 10ms
cargo run --bin rsx-chaos-test -- \
  --scenario dual-me-crash \
  --symbol BTCUSD

# Verify:
# - Data loss <= 10ms orders
# - Fills not lost (WAL available)
# - Recovery time <= 20s
# - Positions consistent after recovery

# Crash Risk master + replica within 10ms
cargo run --bin rsx-chaos-test -- \
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
# Simulate disk full
cargo run --bin rsx-chaos-test -- \
  --scenario disk-full \
  --component ME

# Verify:
# - ME stalls on WAL buffer full
# - No silent data loss
# - Alert fires (disk full)
# - Recovery after disk space freed

# Simulate disk slow (inject latency)
cargo run --bin rsx-chaos-test -- \
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
# Partition Risk <-> ME for 5min
cargo run --bin rsx-chaos-test -- \
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

```sql
-- Check advisory locks per shard
SELECT objid AS shard_id, COUNT(*) AS lock_count, array_agg(pid) AS holder_pids
FROM pg_locks
WHERE locktype = 'advisory'
GROUP BY objid
ORDER BY objid;

-- Expected: Each shard has exactly 1 lock
```

### 7.2 Position Reconciliation

```sql
-- Quick reconciliation check (sample 1%)
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

-- Expected: 0 rows (all match)
```

### 7.3 Tips Monotonic Check

```sql
-- Verify tips never decreased
SELECT
  instance_id,
  symbol_id,
  last_seq,
  updated_at,
  LAG(last_seq) OVER (PARTITION BY instance_id, symbol_id ORDER BY updated_at) AS prev_seq
FROM tips
WHERE last_seq < LAG(last_seq) OVER (PARTITION BY instance_id, symbol_id ORDER BY updated_at);

-- Expected: 0 rows (tips always increase)
```

### 7.4 Funding Zero-Sum Check

```sql
-- Verify funding payments sum to zero per interval
SELECT
  symbol_id,
  settlement_ts,
  SUM(amount) AS total_funding
FROM funding_payments
GROUP BY symbol_id, settlement_ts
HAVING ABS(SUM(amount)) > 100;  -- Allow small rounding error

-- Expected: 0 rows (all zero-sum)
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
