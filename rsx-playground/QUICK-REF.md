# Quick Reference - RSX Playground Fixes

## Start Server
```bash
cd /home/onvos/sandbox/rsx/rsx-playground
uv run server.py
# Open: http://localhost:49171
```

## What Was Fixed

| # | Issue | Fix | Test |
|---|-------|-----|------|
| 1 | Docs URL | Already correct (krons.cx) | Click footer link |
| 2 | Create user broken | Changed to /api/users/create | Risk tab → Create User |
| 3 | WALs not shown | Recursive directory scan | WAL tab → see pengu/10 files |
| 4 | Orders not arriving | WebSocket to Gateway | Orders tab → Submit |
| 5 | Latencies broken | Percentile tracking | Risk tab → see p50/p95/p99 |
| 6 | Scenario switching | Auto-restart processes | Control → Switch Scenario |
| 7 | WAL dump error | Fixed path construction | WAL tab → Dump JSON |
| 8 | Graphs broken | Regression vs baseline | Verify tab → Latency |

## Key Code Locations

### Order Submission Flow
```
server.py:1146  send_order_to_gateway()  # WebSocket client
server.py:1173  api_orders_test()         # Form handler
pages.py:897    orders_page()             # Order form UI
```

### Latency Tracking
```
server.py:363   order_latencies           # Global list
server.py:1215  # Latency recorded after order
pages.py:1770   render_risk_latency()     # Display percentiles
pages.py:1608   render_latency_regression() # vs baseline
```

### WAL Files
```
server.py:445   scan_wal_files()          # Recursive scan
server.py:1449  api_wal_dump()            # Dump endpoint
```

### User Creation
```
server.py:1392  api_create_user()         # POST /api/users/create
pages.py:430    # Create User button
```

## Quick Tests

### Test Order Submission
```bash
# 1. Start Gateway
cd /home/onvos/sandbox/rsx
./start minimal

# 2. Submit order via UI
# Orders tab → Fill form → Submit

# 3. Check logs
tail -f log/gateway*.log
```

### Test Create User
```bash
# 1. Verify DB connection
psql postgresql://postgres:postgres@10.0.2.1:5432/rsx_dev -c "\dt"

# 2. Click Create User on Risk tab

# 3. Verify user created
psql postgresql://postgres:postgres@10.0.2.1:5432/rsx_dev \
  -c "SELECT * FROM users ORDER BY user_id DESC LIMIT 1;"
```

### Test WAL Display
```bash
# 1. Check WAL files exist
find /home/onvos/sandbox/rsx/tmp/wal -name "*.wal"

# 2. Navigate to WAL tab in UI
# Should see files from pengu/10/ and mark/100/

# 3. Click "Dump JSON"
# Should show WAL records
```

### Test Latency Tracking
```bash
# 1. Submit 10 orders (use Batch button)
# 2. Risk tab → Risk Check Latency
# Should show:
#   p50: Xus
#   p95: Yus
#   p99: Zus
#   max: Mus
#   n=10
```

## Expected Latencies
- **Gateway → Risk → ME → Gateway**: Target <50us
- **ME Match**: Target <500ns (0.5us)
- **Color Coding**:
  - Green: <100us (good)
  - Amber: 100-500us (acceptable)
  - Red: >=500us (slow)

## Database Connection
```bash
# Default (configured in server.py)
postgresql://postgres:postgres@10.0.2.1:5432/rsx_dev

# Test connection
psql $DB_URL -c "SELECT version();"
```

## Common Issues

### "websockets library not installed"
```bash
cd rsx-playground
uv pip install websockets
```

### "gateway not running"
```bash
# Check Gateway process
ps aux | grep rsx-gateway
netstat -tulpn | grep 8080

# Start if needed
cd /home/onvos/sandbox/rsx
./start minimal
```

### "postgres not connected"
```bash
# Check PostgreSQL
systemctl status postgresql
psql postgresql://postgres:postgres@10.0.2.1:5432/rsx_dev -c "\l"
```

### WAL files not showing
```bash
# Verify files exist
ls -la /home/onvos/sandbox/rsx/tmp/wal/*/

# Check permissions
stat /home/onvos/sandbox/rsx/tmp/wal
```

## File Paths
```
/home/onvos/sandbox/rsx/
  rsx-playground/
    server.py         # Main server
    pages.py          # UI templates
    requirements.txt  # Dependencies
    FIXES-APPLIED.md  # Detailed fixes
    TEST-PLAN.md      # Full test guide
    SUMMARY.md        # Implementation summary
    QUICK-REF.md      # This file
  tmp/wal/            # WAL files
    pengu/10/         # PENGU symbol WAL
    mark/100/         # Mark price WAL
  target/debug/
    rsx-cli           # WAL dump tool
```

## API Endpoints Changed
- `POST /api/users` → `POST /api/users/create`
- `GET /x/risk-latency` → now accepts latency data
- `GET /x/latency-regression` → now accepts latency data
- `POST /api/scenario/switch` → now restarts processes

## New Functions
- `send_order_to_gateway()` - WebSocket client
- `render_risk_latency(latencies)` - Percentile display
- `render_latency_regression(latencies)` - Baseline comparison
- Enhanced `scan_wal_files()` - Recursive scan

## Verify All Fixes
```bash
cd /home/onvos/sandbox/rsx/rsx-playground
python3 -m py_compile server.py pages.py  # No errors
uv run server.py  # Start server
# Visit http://localhost:49171
# Check all 8 items in TEST-PLAN.md
```
