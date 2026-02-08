# RSX Crash Scenario Analysis

Comprehensive analysis of crash scenarios with detailed failure modes, recovery
paths, data loss calculations, and verification procedures. This document
complements [GUARANTEES.md](GUARANTEES.md) with specific scenario walkthroughs.

**Relationship to GUARANTEES.md:**
- GUARANTEES.md defines formal bounds (0ms fills, 10ms orders, 100ms positions)
- This document proves those bounds hold via concrete scenarios
- Each scenario verifies data loss <= guaranteed bounds
- Recovery procedures demonstrate RTO targets from GUARANTEES.md

**Scope:** All credible failure scenarios affecting data durability or system
availability. Each scenario includes preconditions, triggers, effects, recovery
steps, and verification.

---

## Scenario Matrix

| ID | Components | Timing | System State | Fills Loss | Orders Loss | Positions Loss | RTO | Verification |
|----|------------|--------|--------------|------------|-------------|----------------|-----|--------------|
| S1 | ME master | During matching | 100 orders in flight | **0ms** | 10ms | 0ms (replay) | 5-10s | Position = sum(fills) |
| S2 | ME master | WAL lag 15ms | 50 orders accepted | **0ms** | 15ms | 0ms (replay) | 5-10s | Check ME seq gaps |
| S3 | Risk master | During fill processing | 20 fills in buffer | **0ms** | 10ms (in-flight) | 10ms | 2-5s | Postgres vs ME fills |
| S4 | Risk master + replica | Both within 50ms | 100 fills in both | **0ms** | 10ms (in-flight) | **100ms** | 30-60s | Full reconciliation |
| S5 | ME + Risk (same shard) | ME first, Risk 1s later | Active trading | **0ms** | 10ms | 10ms | 10-30s | Cross-check WAL vs tips |
| S6 | Postgres master | During batch flush | 500 position updates | **0ms** | 0ms | 10ms (uncommitted) | 30-60s | Check PG WAL vs buffer |
| S7 | ME disk | Total disk loss | Mid-trading | **0ms** | 10ms | 0ms (replay) | 20-60s | Replica promotion |
| S8 | Risk ↔ ME partition | 5min partition | ME continues | **0ms** | 10ms (in-flight) | 0ms (buffered) | 5min + 30s | Replay 5min fills |
| S9 | Slow consumer: Risk | Lag >100ms | ME backpressure | **0ms** | 0ms (rejected) | 0ms (stalled) | N/A (perf) | Verify ME stall |
| S10 | Split-brain: Dual Risk | Both think main | Dual processing | **0ms** | Unbounded | Unbounded | Manual | Check lock state |
| S11 | ME master + WAL corruption | Corrupted WAL file | 1000 fills in file | **Unbounded** | Unbounded | Unbounded | Manual | Restore from backup |
| S12 | Gateway crash | During order submission | 50 orders in flight | **0ms** | Unbounded | 0ms | <1s | Users resubmit |
| S13 | MARKETDATA crash | During L2 broadcast | Shadow book | **0ms** | N/A | 0ms | <1s | Rebuild from WAL |
| S14 | ME + Risk + Postgres | All crash in 10ms | Full system | **0ms** | 10ms | **100ms** | 60-180s | Full system verify |
| S15 | Network partition: All | Full isolation 1min | No cross-component | **0ms** | 10ms | 0ms (buffered) | 1min + 60s | Full replay |
| S16 | ME WAL rotation | During file rotation | Active file incomplete | **0ms** | 10ms | 0ms | 20-60s | Rename + restart |
| S17 | DXS buffer overflow | Consumer lag >10min | Hot WAL expired | **0ms** | 10ms | 0ms | 5-30min | ARCHIVE replay |
| S18 | Config update | Mid-crash | Config event lost | **0ms** | 10ms | 0ms | <5s | Reapply config |
| S19 | Funding settlement | Mid-iteration | Partial funding | **0ms** | 10ms | 0ms | 30s | Resume from checkpoint |

---

## S1: ME Master Crash During Matching

### Preconditions
- ME processing 1000 orders/sec
- 100 orders in various stages (pre-match check, matching, fill emission)
- WAL flush last occurred 7ms ago (next flush in 3ms)
- Risk, Gateway, Postgres all healthy

### Trigger
- ME process crashes (OOM, panic, kill -9, kernel panic)

### Immediate Effect
- ME stops responding to order requests
- Risk detects heartbeat timeout after 5s
- Gateway detects connection timeout after 5s
- 100 orders in ME in-flight buffers lost

### Data at Risk
- **Orders:** 100 orders in ME buffers + up to 70 orders accepted in last 7ms (not yet flushed to WAL)
- **Fills:** 0 fills at risk (all fills are written to WAL buffer immediately, flushed within 10ms)
- **Positions:** 0 at risk (Risk replays from ME WAL)

### Detection
```bash
# ME heartbeat timeout
curl http://me:9100/health
# Connection refused or timeout

# Risk logs show connection error
journalctl -u rsx-risk@shard0 -n 10 | grep "ME connection"
# "ME connection lost for symbol BTCUSD"

# Gateway logs show order timeout
journalctl -u rsx-gateway -n 10 | grep "timeout"
# "Order routing timeout for symbol BTCUSD"

# Alert fires
# "ME heartbeat timeout for symbol BTCUSD (P1)"
```

### Recovery Steps

```bash
# 1. Verify crash (not just network blip)
systemctl status rsx-matching@BTCUSD
# Expected: "failed" or "inactive"

# 2. Check for core dump (for postmortem)
ls -lt /var/crash/
# If core dump exists, save for analysis

# 3. Check if replica healthy
curl http://me-replica:9100/health
# Expected: {"status": "ok", "role": "replica", "seq": 12345670}

# 4. If replica healthy, promote replica (fastest path)
#    Update service discovery to point to replica
consul kv put matching/BTCUSD/master me-replica:9100

# 5. Restart crashed master (becomes new replica)
systemctl restart rsx-matching@BTCUSD

# 6. Wait for master recovery
tail -f /var/log/rsx/matching-BTCUSD.log
# Watch for: "Loaded snapshot at seq=12345000"
# Watch for: "Replayed WAL from seq=12345001 to seq=12345678"
# Watch for: "Recovery complete, seq=12345678"

# 7. Verify ME recovered
curl http://me:9100/health
# Expected: {"status": "ok", "role": "master", "seq": 12345678}

# 8. Check WAL for gaps
cargo run --bin rsx-wal-check -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --from-seq 12345000
# Expected: "No gaps detected"

# 9. Verify Risk replayed any missed fills
psql -c "SELECT last_seq FROM tips WHERE symbol_id = 1;"
# Should match ME seq (within 10ms lag)

# 10. Run position reconciliation
cargo run --bin rsx-position-reconcile -- --symbol-id 1
# Expected: "All positions match fills"

# 11. Resume trading
consul kv put matching/BTCUSD/status active
# Gateway resumes routing orders to ME
```

### Data Loss Calculation

**Orders:**
- In-flight orders in ME buffers: 100 (lost, users must resubmit)
- Orders accepted in last 7ms before crash: up to 70 (not yet flushed to WAL, lost)
- **Total:** up to 170 orders lost (100 buffer + 70 unflushed)
- **Bound:** 10ms flush window = up to 100 orders/sec * 0.01s = 10 orders max unflushed (if order rate is sustained). The 170 total is burst scenario.

**Fills:**
- All fills emitted by ME are written to WAL buffer immediately (in-memory append)
- WAL buffer flushed to disk every 10ms OR 1000 records (whichever first)
- Last WAL flush was 7ms ago, so fills from [now-7ms, now] are in buffer, not yet flushed
- On crash: unflushed fills in buffer are LOST from this ME instance
- BUT: ME replica receives same fills via SPSC ring and has flushed to its own WAL
- **Total:** 0 fills lost system-wide (replica WAL has all fills)
- **Proof:** Risk replays from surviving ME replica WAL via DXS. All fills with seq <= last_flushed_seq on replica are durable.

**Positions:**
- Risk applies fills from ME WAL via DXS replay
- Any fills Risk missed (during ME downtime) are replayed from WAL
- **Total:** 0 position drift
- **Proof:** Position = sum(fills where seq <= tips[symbol_id])

### Recovery Time

**Best case:** 3s (replica promotion, no master restart)
**Typical:** 5s-10s (replica promotion + master restart as new replica)
**Worst case:** 20s (replica also unhealthy, cold start from snapshot)

### Verification

```sql
-- 1. No seq gaps in fills
WITH seq_check AS (
  SELECT
    symbol_id,
    seq,
    LAG(seq) OVER (PARTITION BY symbol_id ORDER BY seq) AS prev_seq
  FROM fills
  WHERE symbol_id = 1
)
SELECT symbol_id, seq, prev_seq, seq - prev_seq AS gap
FROM seq_check
WHERE seq - prev_seq > 1;

-- Expected: 0 rows

-- 2. All positions match fills
WITH fills_sum AS (
  SELECT
    taker_user_id AS user_id,
    symbol_id,
    SUM(CASE WHEN side = 0 THEN qty ELSE 0 END) AS fills_buy,
    SUM(CASE WHEN side = 1 THEN qty ELSE 0 END) AS fills_sell
  FROM fills
  WHERE symbol_id = 1
  GROUP BY taker_user_id, symbol_id
)
SELECT COUNT(*)
FROM positions p
JOIN fills_sum f ON p.user_id = f.user_id AND p.symbol_id = f.symbol_id
WHERE p.long_qty != f.fills_buy OR p.short_qty != f.fills_sell;

-- Expected: 0

-- 3. Tips advanced correctly
SELECT last_seq FROM tips WHERE symbol_id = 1;
-- Expected: >= 12345678 (ME's current seq)
```

### Lessons
- Fills are NEVER lost (WAL guarantees 0ms loss)
- Orders CAN be lost (users must implement timeout-resubmit)
- Replica promotion is fastest recovery (3s vs 10s cold start)
- Position reconciliation catches any replay bugs

---

## S4: Risk Master + Replica Crash (Correlated Failure)

### Preconditions
- Both Risk master and replica running healthy
- 100 fills/sec being processed
- Last Postgres flush was 8ms ago (next flush in 2ms)
- Both instances have buffered fills not yet in Postgres

### Trigger
- Datacenter power loss, kernel panic, or bug causing both to crash
- Both crash within 50ms window (before Postgres flush completes)

### Immediate Effect
- No Risk instance running
- Advisory lock released (Postgres connection dropped)
- Gateway cannot send orders (connection refused)
- 100 fills in master's in-memory buffer (not yet in Postgres)
- 100 fills in replica's buffer (same fills)

### Data at Risk
- **Fills:** 0 at risk (ME has all fills in WAL)
- **Positions:** Up to 100ms of position updates (worst case)
- **Postgres:** Last committed batch was 10ms ago, so 10ms of updates at risk

### Detection
```bash
# Both instances heartbeat timeout
curl http://risk-master:9200/health
curl http://risk-replica:9200/health
# Both: Connection refused

# Advisory lock released
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 0 (P0 alert!)

# Gateway logs
journalctl -u rsx-gateway -n 10 | grep "Risk"
# "Risk connection lost for shard 0"

# Alert fires
# "No main for shard 0 (P0)"
```

### Recovery Steps

```bash
# 1. Verify both instances down
systemctl status rsx-risk@shard0
systemctl status rsx-risk-replica@shard0
# Both: "failed" or "inactive"

# 2. Restart master first (to acquire lock)
systemctl start rsx-risk@shard0

# 3. Wait for master to acquire advisory lock
sleep 2
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1

# 4. Check master loaded positions from Postgres
journalctl -u rsx-risk@shard0 -n 100 | grep "Loaded"
# "Loaded 10000 positions from Postgres"

# 5. Check master requested DXS replay from ME
journalctl -u rsx-risk@shard0 -n 100 | grep "DXS"
# "Requesting DXS replay for symbol 1 from seq=12345670"
# (seq from tips table in Postgres)

# 6. Wait for CaughtUp from all symbols
timeout 60s journalctl -u rsx-risk@shard0 -f | grep -m 1 "CaughtUp"
# "CaughtUp for all symbols, going live"

# 7. Verify tips advanced
psql -c "SELECT symbol_id, last_seq, updated_at FROM tips WHERE instance_id = 0 ORDER BY symbol_id;"
# Should show recent updated_at (within last 10s)

# 8. Start replica (will buffer fills, not acquire lock)
systemctl start rsx-risk-replica@shard0

# 9. Verify replica did NOT acquire lock
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1 (master only)

# 10. Run full position reconciliation (critical!)
cargo run --bin rsx-position-reconcile -- --shard-id 0 --verbose
# Expected: "All positions match fills"

# If any mismatch:
# - Master may have applied fills after last Postgres commit
# - Replay from ME fills the gap
# - Postgres state should now match ME fills exactly
```

### Data Loss Calculation

**Fills:**
- ME has all fills in WAL (ME replica flushed within 10ms)
- Risk replays from ME WAL via DXS
- **Total:** 0 fills lost
- **Proof:** DXS replay serves fills from ME replica WAL. ME replica flushed to disk before both Risk instances crashed. Risk deduplicates by seq on replay.

**Positions:**
- Master's in-memory buffer had ~80 fills (8ms of processing at 100 fills/sec)
- Replica's buffer also had ~80 fills (same seq range, buffered not applied)
- Last Postgres commit was 10ms ago (batch flush interval)
- **Typical case:** 10ms of position updates not yet committed to Postgres
- **Worst case:** 100ms of position updates lost (if Postgres transaction in progress + both Risk crash before commit)
- **Replay:** New Risk instance requests from `tips[symbol_id] + 1`, replays all missed fills from ME WAL
- After replay: positions = sum(fills) exactly

**Bound proof:**
- Risk flushes to Postgres every 10ms OR 1000 records (whichever first)
- Risk buffer has up to 10ms of uncommitted position updates when both crash
- Postgres commit latency under load: up to 90ms (checkpoint/vacuum contention)
- Total worst case: 10ms (Risk buffer) + 90ms (Postgres commit lag) = 100ms
- This is acceptable per GUARANTEES.md (dual component failure = 100ms loss bound)

### Recovery Time

**Best case:** 10s (master starts, replays 1s of fills, goes live)
**Typical:** 30s-60s (master replays 10s of fills, verifies positions)
**Worst case:** 120s (large replay gap, slow Postgres)

### Verification

```sql
-- 1. Advisory lock held by exactly 1 instance
SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;
-- Expected: 1

-- 2. All positions match fills
WITH fills_sum AS (
  SELECT
    taker_user_id AS user_id,
    symbol_id,
    SUM(CASE WHEN side = 0 THEN qty ELSE 0 END) AS fills_buy,
    SUM(CASE WHEN side = 1 THEN qty ELSE 0 END) AS fills_sell
  FROM fills
  WHERE taker_user_id % 16 = 0  -- Shard 0 owns user_id % 16 = 0
  GROUP BY taker_user_id, symbol_id
)
SELECT
  p.user_id,
  p.symbol_id,
  p.long_qty,
  f.fills_buy,
  p.short_qty,
  f.fills_sell
FROM positions p
JOIN fills_sum f ON p.user_id = f.user_id AND p.symbol_id = f.symbol_id
WHERE p.long_qty != f.fills_buy OR p.short_qty != f.fills_sell;

-- Expected: 0 rows

-- 3. Tips monotonic (never decreased)
WITH tip_history AS (
  SELECT
    symbol_id,
    last_seq,
    LAG(last_seq) OVER (PARTITION BY symbol_id ORDER BY updated_at ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS prev_seq
  FROM tips
  WHERE instance_id = 0
)
SELECT symbol_id, last_seq, prev_seq
FROM tip_history
WHERE prev_seq IS NOT NULL AND last_seq < prev_seq;

-- Expected: 0 rows

-- 4. Margin recalculated correctly
-- (Run on Risk instance, compares incremental vs from-scratch)
curl http://risk:9200/api/v1/margin-verify?user_id=123
-- Expected: {"status": "ok", "drift": 0}
```

### Lessons
- **100ms loss bound holds:** Dual crash = max 100ms position updates lost
- **Fills never lost:** ME WAL + DXS replay provides complete history
- **Advisory lock prevents split-brain:** Only 1 instance acquires lock
- **Replay is deterministic:** Replaying fills produces identical positions
- **Verification is critical:** Must run reconciliation after dual crash

---

## S8: Network Partition (Risk ↔ ME) for 5 Minutes

### Preconditions
- Risk and ME both healthy
- Network link between Risk and ME fails (router crash, cable cut)
- ME continues matching (has local WAL)
- Risk cannot receive fills (connection timeout)

### Trigger
- Network partition between Risk datacenter and ME datacenter
- Partition lasts 5 minutes

### Immediate Effect
- Risk detects ME connection timeout after 5s
- Risk stops receiving fills (in-flight fills lost in SPSC ring)
- ME continues matching, writes fills to WAL
- ME's DXS replay buffer accumulates 5min of fills
- Gateway continues accepting orders (unaware of partition)
- Users' orders reach ME, fills emitted, but Risk doesn't see them

### Data at Risk
- **Fills:** 0 at risk (ME WAL retains all fills for 10min)
- **Positions:** 0 at risk (Risk will replay from ME WAL after partition heals)
- **Orders in flight:** Unbounded (if partition during order routing, lost)

### Detection
```bash
# Risk logs show ME connection timeout
journalctl -u rsx-risk@shard0 -f | grep "ME connection"
# "ME connection lost for symbol BTCUSD"
# "Attempting reconnect to ME..."

# ME logs show no change (continues matching)
journalctl -u rsx-matching@BTCUSD -f | grep "fill"
# "Emitted fill seq=12345678"
# "Emitted fill seq=12345679"

# Network check
ping me-host
# From Risk host: "Destination host unreachable"

# Alert fires
# "Risk cannot connect to ME for symbol BTCUSD (P1)"
```

### During Partition (ME Behavior)

ME continues normally:
- Accepts orders from Gateway (if Gateway → ME link intact)
- Matches orders, emits fills
- Writes fills to WAL (flushed every 10ms)
- DXS replay buffer accumulates fills (in-memory ring)
- If Risk's SPSC ring full (cannot push): ME stalls
  - But if partition = no connection, ring push fails immediately
  - ME logs error, continues (fill is in WAL, can replay)

### During Partition (Risk Behavior)

Risk detects connection loss:
- Stops receiving fills from ME
- Positions become stale (last fill received was 5min ago)
- Margin calculations use stale positions (risk of incorrect liquidations!)
- Gateway orders queued (if Risk → ME link intact) or rejected
- Risk continues trying to reconnect (exponential backoff)

### Partition Heals

```bash
# Network link restored
ping me-host
# "icmp_seq=1 ttl=64 time=1.2ms"

# Risk reconnects to ME
journalctl -u rsx-risk@shard0 -f | grep "Reconnected"
# "Reconnected to ME for symbol BTCUSD"

# Risk requests DXS replay from last tip
journalctl -u rsx-risk@shard0 -f | grep "DXS"
# "Requesting DXS replay for symbol 1 from seq=12345678"

# ME serves replay from WAL
journalctl -u rsx-matching@BTCUSD -f | grep "DXS"
# "Serving DXS replay for symbol 1 from seq=12345678 to seq=12375678"
# (30,000 fills = 5min at 100 fills/sec)

# Risk processes replay
journalctl -u rsx-risk@shard0 -f | grep "Replay"
# "Replayed 30000 fills for symbol 1"
# "CaughtUp for symbol 1"

# Positions updated
curl http://risk:9200/api/v1/positions?user_id=123
# Shows current positions (including 5min of fills)
```

### Recovery Steps

```bash
# 1. Verify partition healed
ping me-host
# Expected: Success

# 2. Check Risk reconnected
systemctl status rsx-risk@shard0
# Expected: "active (running)"

journalctl -u rsx-risk@shard0 -n 20 | grep "ME"
# "Reconnected to ME for symbol BTCUSD"

# 3. Check DXS replay completed
journalctl -u rsx-risk@shard0 -n 100 | grep "CaughtUp"
# "CaughtUp for symbol 1"

# 4. Verify tips advanced
psql -c "SELECT symbol_id, last_seq, updated_at FROM tips WHERE symbol_id = 1;"
# Should show recent updated_at and seq matching ME

# 5. Run position reconciliation
cargo run --bin rsx-position-reconcile -- --symbol-id 1
# Expected: "All positions match fills"

# 6. Check for any liquidations during stale period
psql -c "SELECT * FROM liquidation_events WHERE timestamp_ns > extract(epoch from now() - interval '10 minutes') * 1e9;"
# Review any liquidations during partition (may have used stale positions)

# 7. Resume normal operations
# (Automatic, no manual intervention needed)
```

### Data Loss Calculation

**Fills:**
- ME wrote all fills to WAL during partition
- DXS replay buffer retained fills (10min retention)
- Risk replayed all fills after partition healed
- **Total:** 0 fills lost
- **Proof:** Risk tips after replay = ME seq (all fills accounted for)

**Positions:**
- During partition: Risk positions stale (5min lag)
- After replay: Risk positions current (= sum of all fills)
- **Total:** 0 position drift
- **Proof:** Position = sum(fills where seq <= tips[symbol_id])

**Orders:**
- During partition: Orders in flight from Gateway → Risk → ME lost
- If partition affects Gateway → Risk: orders rejected at Gateway (immediate feedback)
- If partition affects Risk → ME only: orders accumulate in Risk → ME ring until full, then rejected
- **Total:** Up to 10ms of orders in-flight when partition starts (per GUARANTEES.md bound). During partition, new orders rejected at Gateway.

### Recovery Time

**Partition duration:** 5min (given)
**Replay time:** 30s (30,000 fills at 1000 fills/sec replay throughput)
**Total downtime:** 5min (partition) + 30s (replay) = 5min 30s

### Verification

```sql
-- 1. No seq gaps during partition
WITH seq_check AS (
  SELECT
    symbol_id,
    seq,
    timestamp_ns,
    LAG(seq) OVER (PARTITION BY symbol_id ORDER BY seq) AS prev_seq
  FROM fills
  WHERE symbol_id = 1
    AND timestamp_ns > extract(epoch from now() - interval '10 minutes') * 1e9
)
SELECT symbol_id, seq, prev_seq, seq - prev_seq AS gap
FROM seq_check
WHERE seq - prev_seq > 1;

-- Expected: 0 rows

-- 2. All positions match fills (including partition period)
-- (Same query as S4, should return 0 rows)

-- 3. Risk tips match ME seq
SELECT
  (SELECT last_seq FROM tips WHERE symbol_id = 1 AND instance_id = 0) AS risk_tip,
  (SELECT seq FROM fills WHERE symbol_id = 1 ORDER BY seq DESC LIMIT 1) AS me_tip;
-- Expected: risk_tip = me_tip (or within 10ms lag)
```

### Lessons
- **DXS replay handles long partitions:** 10min retention covers most scenarios
- **Positions always reconstructed from fills:** No drift even after 5min partition
- **Stale positions during partition:** Risk calculations may be incorrect (liquidations?)
- **Partition >10min:** Risk must rebuild from snapshot + full WAL (longer recovery)
- **Gateway backpressure:** If Risk stalls, Gateway rejects orders (prevents unbounded queue)

---

## S10: Split-Brain (Both Risk Instances Think They Are Main)

### Preconditions
- Risk master and replica both running
- Postgres available but experiencing network partition
- Master's connection to Postgres drops
- Replica's connection to Postgres intact

### Trigger
- Network partition between master and Postgres
- Master's advisory lock lease expires (connection timeout)
- Postgres releases master's advisory lock
- Replica polls for lock, acquires it (thinks main is dead)
- Master reconnects to Postgres (network heals)
- Master tries to renew lease, succeeds (Postgres bug or race condition)

**NOTE:** This should NEVER happen if advisory locks work correctly. This is a
defensive scenario to detect if it somehow occurs.

### Immediate Effect
- Both master and replica hold advisory lock (Postgres bug)
- Both instances process fills and write to Postgres
- Dual writes to Postgres (positions updated by both)
- Risk of position corruption (if both apply same fill)

### Detection
```sql
-- Advisory lock count = 2 (CRITICAL!)
SELECT objid AS shard_id, COUNT(*) AS lock_count, array_agg(pid) AS holder_pids
FROM pg_locks
WHERE locktype = 'advisory'
GROUP BY objid;

-- Expected: shard 0 has lock_count = 2 (P0 ALERT!)
```

```bash
# Alert fires
# "Split-brain detected for shard 0: 2 instances hold lock (P0)"

# Check both instances
curl http://risk-master:9200/health
# {"status": "ok", "role": "main", "shard_id": 0}

curl http://risk-replica:9200/health
# {"status": "ok", "role": "main", "shard_id": 0}
# Both think they are main!
```

### Immediate Actions

```bash
# 1. HALT both instances immediately (prevent further corruption)
systemctl stop rsx-risk@shard0
systemctl stop rsx-risk-replica@shard0

# 2. Verify locks released
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 0

# 3. Identify which instance has newer state
# Note: instance_id convention: shard 0 master=0, replica=100
#                                shard 1 master=1, replica=101, etc.
#       Adjust based on actual deployment
psql -c "
SELECT instance_id, symbol_id, last_seq, updated_at
FROM tips
WHERE instance_id IN (0, 100)  -- master=0, replica=100 for shard 0
ORDER BY symbol_id, updated_at DESC;
"

# Example output:
# instance_id | symbol_id | last_seq  | updated_at
# ------------|-----------|-----------|---------------------------
# 100         | 1         | 12345700  | 2024-01-15 10:05:32.123
# 0           | 1         | 12345680  | 2024-01-15 10:05:31.987
# 100         | 2         | 23456700  | 2024-01-15 10:05:32.456
# 0           | 2         | 23456690  | 2024-01-15 10:05:31.654

# Replica (100) has higher tips = more recent state
# Keep replica's tips, discard master's

# 4. Check for duplicate fills in Postgres
psql -c "
SELECT symbol_id, user_id, seq, COUNT(*) AS dup_count
FROM fills
WHERE symbol_id = 1
GROUP BY symbol_id, user_id, seq
HAVING COUNT(*) > 1;
"

# If duplicates exist:
# - Both instances wrote same fill to Postgres
# - Need to deduplicate (keep one, delete others)

# 5. Check for position corruption
psql -c "
SELECT p.user_id, p.symbol_id, p.long_qty, p.short_qty, p.version
FROM positions p
WHERE p.symbol_id = 1
  AND p.user_id % 16 = 0  -- Shard 0 users
ORDER BY p.user_id, p.symbol_id;
"

# Manual inspection required:
# - Compare positions with sum(fills)
# - If mismatch: rebuild from fills
```

### Recovery Steps

```bash
# 6. Choose which instance to keep (higher tips = newer)
# Let's say replica has newer state, keep replica

# 7. Discard master's tips (if lower)
psql -c "DELETE FROM tips WHERE instance_id = 0;"
# WARNING: Only if replica has higher tips for ALL symbols!

# 8. Restart replica as main
systemctl start rsx-risk-replica@shard0

# 9. Verify replica acquired lock
sleep 2
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1

# 10. Restart master as new replica (will NOT acquire lock)
systemctl start rsx-risk@shard0

# 11. Verify only 1 lock held
psql -c "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;"
# Expected: 1

# 12. Deduplicate fills in Postgres (if duplicates found)
psql -c "
DELETE FROM fills
WHERE ctid NOT IN (
  SELECT MIN(ctid)
  FROM fills
  GROUP BY symbol_id, seq
);
"

# 13. Rebuild all positions from fills (to be safe)
cargo run --bin rsx-position-rebuild -- --shard-id 0 || {
  echo "CRITICAL: Position rebuild failed!"
  exit 1
}

# This will:
# - Truncate positions table for shard 0
# - Read all fills from Postgres
# - Recompute positions from scratch
# - Write corrected positions back

# 14. Verify positions match fills
cargo run --bin rsx-position-reconcile -- --shard-id 0 || {
  echo "CRITICAL: Positions still do not match fills after rebuild!"
  echo "Check fills table integrity, ME WAL consistency, and deduplication logic"
  exit 1
}
# Expected: "All positions match fills"

# 15. Resume trading
# (Manual approval required after P0 incident)
```

### Data Loss Calculation

**Fills:**
- Both instances may have written same fill to Postgres
- Duplicate fills in Postgres (deduped by seq in step 12)
- **Total:** 0 fills lost (duplicates removed, all unique fills preserved)
- **Proof:** Fills deduplicated by (symbol_id, seq), ME WAL is source of truth

**Positions:**
- If both instances applied same fill: position may be doubled
- If fills applied out of order: position may be incorrect
- **Total:** Unbounded position corruption (manual rebuild required)
- **Proof:** Rebuild from fills (position = sum(fills)) corrects all errors

**Postgres integrity:**
- Duplicate fills: removed by dedup query
- Corrupted positions: rebuilt from fills
- Tips: keep newer instance's tips, discard older

### Recovery Time

**Manual intervention required:** 30min-2hr (depends on data size)
**Automated rebuild:** 10min-30min (position rebuild from fills)

### Verification

```sql
-- 1. Only 1 lock held
SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND objid = 0;
-- Expected: 1

-- 2. No duplicate fills
SELECT symbol_id, seq, COUNT(*) AS dup_count
FROM fills
WHERE symbol_id = 1
GROUP BY symbol_id, seq
HAVING COUNT(*) > 1;
-- Expected: 0 rows

-- 3. All positions match fills
-- (Same query as S4, should return 0 rows after rebuild)

-- 4. Tips monotonic
-- (Same query as S4, should return 0 rows)

-- 5. Margin consistent
curl http://risk:9200/api/v1/margin-verify?user_id=123
-- Expected: {"status": "ok", "drift": 0}
```

### Lessons
- **Advisory locks must be exclusive:** Postgres should NEVER allow 2 holders
- **Monitoring critical:** Must alert on lock_count != 1 immediately
- **Halt on detection:** Prevent further corruption before manual fix
- **Rebuild from fills:** Always possible, fills are source of truth
- **Root cause:** Investigate Postgres bug, network partition logic, lease renewal

### Mitigation
- Implement heartbeat between master and replica (detect split-brain faster)
- Add epoch/generation number to writes (detect stale writes)
- Test advisory lock behavior under network partition
- Add assertion in code: `SELECT COUNT(*) FROM pg_locks WHERE objid = shard_id; assert(count == 1);`

---

## S15: Full Network Partition (All Components Isolated) for 1 Minute

### Preconditions
- All components healthy and processing
- Datacenter network failure (core router crash)
- All cross-component links fail simultaneously

### Trigger
- Network partition isolates all components from each other
- Partition lasts 1 minute

### Immediate Effect
- Gateway ↔ Risk: Gateway cannot send orders
- Risk ↔ ME: Risk cannot receive fills
- Risk ↔ Postgres: Risk cannot flush positions
- ME ↔ Postgres: No direct link (ME uses local WAL)
- All components continue running in isolation

### During Partition

**Gateway:**
- Detects Risk connection timeout (5s)
- Rejects all new orders (connection error)
- Users see "Service unavailable" errors
- No data loss (stateless)

**Risk:**
- Detects ME connection timeout (5s)
- Detects Postgres connection timeout (5s)
- Stops processing (no fills from ME, cannot flush to Postgres)
- Buffers any pending writes in memory
- Advisory lock held (Postgres connection still open, lock released only on connection drop or explicit release)

**ME:**
- Continues matching (has local WAL)
- Writes fills to WAL (flushed every 10ms)
- Cannot push fills to Risk (SPSC ring or network)
- Fills accumulate in DXS replay buffer (10min retention)
- No data loss (WAL is local)

**Postgres:**
- Continues running (no clients connected during partition)
- Advisory locks held by Risk (connection still open from Postgres perspective)
- If TCP keepalive times out (typically 60s-120s): Postgres detects dead connection, releases lock

### Partition Heals

```bash
# Network restored
ping risk-host
ping me-host
ping postgres-host
# All: Success

# Gateway reconnects to Risk
journalctl -u rsx-gateway -f | grep "Risk"
# "Reconnected to Risk shard 0"

# Risk reconnects to ME
journalctl -u rsx-risk@shard0 -f | grep "ME"
# "Reconnected to ME for symbol BTCUSD"

# Risk reconnects to Postgres
journalctl -u rsx-risk@shard0 -f | grep "Postgres"
# "Reconnected to Postgres"

# Risk requests DXS replay from ME
journalctl -u rsx-risk@shard0 -f | grep "DXS"
# "Requesting DXS replay for symbol 1 from seq=12345678"

# ME serves replay (1min of fills at 100/sec = 6000 fills)
journalctl -u rsx-matching@BTCUSD -f | grep "DXS"
# "Serving DXS replay for symbol 1 from seq=12345678 to seq=12351678"

# Risk processes replay
journalctl -u rsx-risk@shard0 -f | grep "Replay"
# "Replayed 6000 fills for symbol 1"
# "CaughtUp for symbol 1"

# Risk flushes buffered writes to Postgres
journalctl -u rsx-risk@shard0 -f | grep "Flush"
# "Flushed 6000 position updates to Postgres"

# Gateway resumes accepting orders
curl -X POST http://gateway/api/v1/orders \
  -d '{"symbol":"BTCUSD","side":"buy","price":50000,"qty":0.1}'
# Expected: {"status": "accepted", "order_id": "..."}
```

### Recovery Steps

```bash
# 1. Verify all components reconnected
ping risk-host && ping me-host && ping postgres-host
# All: Success

# 2. Check Gateway processing orders
curl http://gateway:8080/health
# Expected: {"status": "ok", "connections": N}

# 3. Check Risk replayed all fills
journalctl -u rsx-risk@shard0 -n 100 | grep "CaughtUp"
# "CaughtUp for all symbols"

# 4. Check Postgres received all position updates
psql -c "SELECT MAX(updated_at) FROM positions;"
# Expected: Recent timestamp (within last 10s)

# 5. Verify tips advanced
psql -c "SELECT symbol_id, last_seq FROM tips WHERE instance_id = 0 ORDER BY symbol_id;"
# Should match ME seq for all symbols

# 6. Run position reconciliation (all shards)
# Note: adjust shard count based on actual deployment (check config)
SHARD_COUNT=$(psql -tAc "SELECT COUNT(DISTINCT instance_id) FROM tips WHERE instance_id < 100;")
for shard in $(seq 0 $((SHARD_COUNT - 1))); do
  cargo run --bin rsx-position-reconcile -- --shard-id $shard || {
    echo "CRITICAL: Shard $shard reconciliation failed!"
    exit 1
  }
done
# Expected: "All positions match fills" for all shards

# 7. Check for any liquidations during partition
psql -c "SELECT * FROM liquidation_events WHERE timestamp_ns > extract(epoch from now() - interval '5 minutes') * 1e9;"
# Review any liquidations during partition (may have used stale prices)

# 8. Resume normal operations
# (Automatic, all components healthy)
```

### Data Loss Calculation

**Fills:**
- ME wrote all fills to local WAL during partition
- DXS replay buffer retained fills (10min > 1min partition)
- Risk replayed all fills after partition healed
- **Total:** 0 fills lost
- **Proof:** All fills in ME WAL, replayed to Risk

**Positions:**
- Risk buffered position updates in memory during partition
- After reconnect: flushed buffered updates to Postgres
- After DXS replay: applied 1min of fills from ME
- **Total:** 0 position drift
- **Proof:** Position = sum(fills where seq <= tips[symbol_id])

**Orders:**
- Orders from users during partition: rejected at Gateway (connection error)
- Orders in flight Gateway → Risk when partition starts: lost (up to 10ms window)
- Orders in flight Risk → ME when partition starts: lost (up to 10ms window)
- **Total:** 10ms of orders lost when partition starts (per GUARANTEES.md bound). During partition, orders rejected at Gateway.

### Recovery Time

**Partition duration:** 1min (given)
**Replay time:** 10s (6000 fills at 600 fills/sec replay rate)
**Total downtime:** 1min (partition) + 10s (replay) = 1min 10s

**Note:** Fill rate during partition is 100 fills/sec (1min = 6000 fills). Replay rate is higher (600 fills/sec) because replaying from WAL is faster than live processing.

### Verification

```bash
# All components healthy
systemctl status rsx-gateway
systemctl status rsx-risk@shard0
systemctl status rsx-matching@BTCUSD
systemctl status postgresql

# All: "active (running)"

# No seq gaps
cargo run --bin rsx-wal-check -- --wal-dir /srv/data/rsx/wal/1 --from-seq 0
# Expected: "No gaps detected"

# Positions match fills (all shards)
SHARD_COUNT=$(psql -tAc "SELECT COUNT(DISTINCT instance_id) FROM tips WHERE instance_id < 100;")
for shard in $(seq 0 $((SHARD_COUNT - 1))); do
  cargo run --bin rsx-position-reconcile -- --shard-id $shard
done
# Expected: "All positions match fills" for all

# Advisory locks held
psql -c "SELECT objid, COUNT(*) FROM pg_locks WHERE locktype = 'advisory' GROUP BY objid ORDER BY objid;"
# Expected: Each shard has exactly 1 lock
```

### Lessons
- **Graceful degradation:** Each component handles partition independently
- **Eventual consistency:** After partition heals, all components converge
- **DXS replay critical:** Handles up to 10min partition (covers most scenarios)
- **User impact:** Orders rejected during partition (expected behavior)
- **No silent data loss:** Fills always replayed, positions always consistent

---

## S16: ME WAL File Rotation Failure During Crash

### Preconditions
- ME writing to active WAL file `{stream_id}_active.wal`
- Current file at 63MB, approaching 64MB rotation threshold
- ME crashes during rotation (after close, before rename)

### Trigger
- ME starts rotating WAL file (closes current, opens new)
- Crash occurs between close and rename with final seq range
- Active file left with temporary name, no seq range in filename

### Immediate Effect
- WAL file `{stream_id}_active.wal` exists but not closed properly
- No final seq range in filename (can't determine last_seq)
- Next ME instance doesn't know where this file fits in seq order

### Recovery Steps

```bash
# 1. List WAL files, identify incomplete rotation
ls -lh /srv/data/rsx/wal/1/
# Find: 1_active.wal (temp file, no seq range)
# Find: 1_12340000_12345000.wal (last completed file)

# 2. Read last seq from active file
cargo run --bin rsx-wal-inspect -- \
  --file /srv/data/rsx/wal/1/1_active.wal \
  --show-last-seq
# Output: "Last valid seq: 12345678"

# 3. Rename active file with correct seq range
mv /srv/data/rsx/wal/1/1_active.wal \
   /srv/data/rsx/wal/1/1_12345001_12345678.wal

# 4. Restart ME, will continue from next seq
systemctl restart rsx-matching@BTCUSD

# 5. Verify WAL integrity
cargo run --bin rsx-wal-check -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --from-seq 12340000
# Expected: "No gaps detected"
```

### Data Loss
- **Fills:** 0ms (all fills in active file are valid, just need rename)
- **Orders:** 10ms (orders accepted but not yet flushed before crash)

### Lessons
- WAL rotation must be atomic (rename is atomic on POSIX)
- Recovery tool needed to inspect and rename incomplete files
- Active file always readable even without final name

---

## S17: DXS Replay Buffer Overflow (Consumer Lag >10min)

### Preconditions
- Risk engine offline for 15 minutes (deploy, maintenance, long crash)
- ME continues matching, writing to WAL
- ME WAL retention = 10min, files older than 10min deleted

### Trigger
- Risk restarts after 15min downtime
- Requests DXS replay from `tips[symbol_id] + 1`
- ME cannot serve: requested seq is in files already deleted

### Recovery Steps

```bash
# 1. Risk detects DXS replay unavailable
journalctl -u rsx-risk@shard0 -n 20 | grep "DXS"
# "DXS replay unavailable: seq 12300000 not in hot WAL"

# 2. Check ARCHIVE for older WAL files
ls -lh /srv/data/rsx/archive/1/
# Find: 1_2024-02-08.wal (yesterday's archive)

# 3. Risk requests from ARCHIVE instead
# (See ARCHIVE.md for replay from cold storage)
cargo run --bin rsx-archive-replay -- \
  --stream-id 1 \
  --from-seq 12300000 \
  --to-seq 12350000 \
  --postgres-uri "$PG_URI"

# This will:
# - Read archive file
# - Insert fills into Postgres fills table
# - Update tips table

# 4. Now Risk can load from Postgres and resume
systemctl restart rsx-risk@shard0

# 5. Risk loads positions + tips from Postgres
# 6. Requests DXS replay from ME (now within 10min window)
# 7. Goes live
```

### Data Loss
- **Fills:** 0ms (ARCHIVE has complete history)
- **Positions:** 0ms (reconstructed from ARCHIVE + hot WAL)
- **Recovery time:** 5-30min (depends on ARCHIVE replay duration)

### Lessons
- DXS hot retention (10min) is insufficient for long outages
- ARCHIVE provides cold storage for infinite history
- Risk must support ARCHIVE fallback when DXS unavailable
- Consider extending hot retention to 1hr for faster recovery

---

## S18: Config Update Mid-Crash (CONFIG_APPLIED Lost)

### Preconditions
- ME receives config update (new fee tiers, margin rates)
- ME applies config, emits `CONFIG_APPLIED` event to WAL
- ME crashes before WAL flush (config event in buffer, not flushed)

### Trigger
- Config update applied at T=0
- WAL flush scheduled for T=10ms
- ME crashes at T=7ms (config event in buffer)

### Immediate Effect
- `CONFIG_APPLIED` event lost (not flushed to WAL)
- ME restart loads old snapshot (pre-config)
- Risk never receives config update
- Orders processed with stale config (wrong fees, wrong margin rates)

### Recovery Steps

```bash
# 1. Detect config mismatch
# Compare ME config version with admin tool
curl http://me:9100/api/v1/config | jq '.version'
# Output: 42 (old)

curl http://admin:8000/api/v1/symbols/BTCUSD/config | jq '.version'
# Output: 43 (new)

# 2. Check if CONFIG_APPLIED event in WAL
cargo run --bin rsx-wal-check -- \
  --wal-dir /srv/data/rsx/wal/1 \
  --event-type CONFIG_APPLIED \
  --from-seq 12345000
# Expected: "No CONFIG_APPLIED events found after seq 12345000"

# 3. Reapply config update to ME
curl -X POST http://me:9100/api/v1/config/apply \
  -H "Content-Type: application/json" \
  -d @config_v43.json
# ME applies, emits CONFIG_APPLIED to WAL, forwards to Risk

# 4. Verify Risk received config update
curl http://risk:9200/api/v1/config | jq '.symbols[].version'
# Expected: all symbols at version 43

# 5. Check for any fills processed with stale config
psql -c "
SELECT COUNT(*) FROM fills
WHERE symbol_id = 1
  AND timestamp_ns BETWEEN $CRASH_TS_NS AND $REAPPLY_TS_NS
  AND (taker_fee != $EXPECTED_FEE OR maker_fee != $EXPECTED_REBATE);
"
# If any found: manual fee adjustment required
```

### Data Loss
- **Config event:** 1 event lost (CONFIG_APPLIED in buffer)
- **Fills with wrong fees:** unbounded (until config reapplied)

### Lessons
- Config updates must be idempotent (reapply safe)
- Risk should validate config version on every fill
- Alert if ME config version lags admin system
- Consider forcing WAL flush after config update (tolerate latency spike)

---

## S19: Funding Settlement Mid-Crash (Partial Application)

### Preconditions
- Funding settlement interval reached (UTC 00:00)
- Risk iterating all positions for BTCUSD, applying funding payments
- 10,000 users with positions, processed 5,000 so far
- Risk crashes mid-iteration

### Trigger
- Funding settlement starts at 00:00:00
- Risk processes users 0-4999, crashes at 00:00:02
- Users 5000-9999 not yet processed

### Immediate Effect
- 5,000 users have funding applied (collateral adjusted, funding_payments row inserted)
- 5,000 users have NOT had funding applied (stale collateral)
- Funding NOT zero-sum (longs paid, but shorts not yet credited)

### Recovery Steps

```bash
# 1. Detect partial funding settlement
psql -c "
SELECT symbol_id, settlement_ts, COUNT(*) AS paid_count
FROM funding_payments
WHERE settlement_ts = '2024-02-08 00:00:00'
GROUP BY symbol_id, settlement_ts;
"
# Output: symbol_id=1, paid_count=5000 (expected 10,000)

# 2. Check last processed user_id
psql -c "
SELECT MAX(user_id) FROM funding_payments
WHERE symbol_id = 1 AND settlement_ts = '2024-02-08 00:00:00';
"
# Output: 4999

# 3. Risk restarts, detects incomplete settlement
# Risk must track last_funded_user_id per symbol per interval
# On startup: check if any interval partially complete

# 4. Risk resumes funding from user_id 5000
# (Risk must be idempotent: check if user already paid before applying)

# 5. Verify funding zero-sum after completion
psql -c "
SELECT symbol_id, settlement_ts, SUM(amount) AS total
FROM funding_payments
WHERE settlement_ts = '2024-02-08 00:00:00'
GROUP BY symbol_id, settlement_ts;
"
# Expected: total = 0 (within rounding error)
```

### Data Loss
- **Funding payments:** 0 (resume from checkpoint)
- **Time to complete:** 30s (replay from user 5000 to 9999)

### Lessons
- Funding settlement must be resumable (checkpoint progress)
- Risk tracks last_funded_user_id in Postgres (funding_checkpoints table)
- On crash: resume from checkpoint, skip already-paid users
- Verify zero-sum after every settlement (invariant check)

---

## Testing Approach

For each scenario above:

1. **Unit tests:** Mock component crashes, verify recovery logic
2. **Integration tests:** Actual component restarts, verify end-to-end recovery
3. **Chaos tests:** Random crashes during load, verify all invariants hold
4. **Partition tests:** Network failure injection, verify replay works
5. **Stress tests:** Sustained load + crashes, verify no degradation

All tests must verify the 8 invariants from GUARANTEES.md:
1. Position = sum(fills)
2. Tips monotonic
3. Margin consistent
4. No negative collateral (unless leverage)
5. Funding zero-sum
6. Fills idempotent
7. Advisory lock exclusive
8. Slab no-leak

---

This document is exhaustive but not complete. New scenarios should be added as
they are discovered through testing, incidents, or design changes.
