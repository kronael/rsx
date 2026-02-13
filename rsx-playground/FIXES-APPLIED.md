# Playground Fixes Applied

## Fixed Issues

### 1. Docs URL (FIXED)
- **Location**: `server.py` line 647 (already fixed)
- **Change**: `http://localhost:8001` → `https://krons.cx/rsx/docs`
- **Status**: Already corrected in codebase

### 2. Create User Endpoint (FIXED)
- **Location**: `server.py` line 1328, `pages.py` line 430
- **Change**: `/api/users` → `/api/users/create`
- **Status**: Endpoint route and HTMX button updated
- **Test**: Click "Create User" button on Risk page

### 3. WAL Files Display (FIXED)
- **Location**: `server.py` scan_wal_files() function
- **Issue**: WAL files in subdirectories (e.g., `tmp/wal/pengu/10/`) not shown
- **Fix**: Added recursive scan for subdirectories
- **Status**: Now scans both top-level and nested WAL files
- **Test**: Check WAL page - should show files from `pengu/10/` and `mark/100/`

### 4. WAL Dump Tool (FIXED)
- **Location**: `server.py` api_wal_dump() function
- **Issue**: Expected Path object but received dict from scan_wal_files()
- **Fix**: Extract path components from dict and construct full path
- **Status**: WAL dump button should work correctly
- **Test**: Click "Dump JSON" button on WAL page

### 5. Order Submission to Gateway (FIXED)
- **Location**: `server.py` new send_order_to_gateway() function
- **Issue**: Orders submitted but not sent to actual Gateway WebSocket
- **Fix**: Added WebSocket client that connects to ws://localhost:8080
- **Features**:
  - Sends order as JSON message to Gateway
  - Waits for OrderAccepted/OrderFailed response
  - Tracks latency in microseconds
  - Graceful fallback if Gateway not running
- **Status**: Orders now actually submitted via WebSocket
- **Test**:
  1. Start Gateway (./start or from Control tab)
  2. Submit test order on Orders page
  3. Should see "accepted" with latency OR "gateway not running"

### 6. Latency Tracking and Display (FIXED)
- **Location**: `server.py` order_latencies global, `pages.py` render_risk_latency()
- **Issue**: Latency functions returned static placeholders
- **Fix**:
  - Track all order latencies in `order_latencies` list
  - Calculate p50/p95/p99/max percentiles
  - Color-code: green (<100us), amber (<500us), red (>=500us)
  - Show sample size (n=X)
- **Additional**: Added latency column to recent orders table
- **Status**: Live latency tracking with percentile display
- **Test**:
  1. Submit several orders
  2. Check Risk page "Risk Check Latency" section
  3. Check Orders page "Recent Orders" for per-order latency

### 7. Scenario Switching (IMPROVED)
- **Location**: `server.py` api_scenario_switch()
- **Issue**: Switched scenario but didn't restart processes
- **Fix**: Now stops all processes, switches scenario, and auto-restarts
- **Status**: Full restart on scenario switch
- **Test**:
  1. Start processes with "minimal" scenario
  2. Switch to "duo" from Control page
  3. Should see processes restart with new configuration

### 8. Latency Regression Display (FIXED)
- **Location**: `pages.py` render_latency_regression()
- **Issue**: Static placeholder
- **Fix**:
  - Calculate p99 latency from actual data
  - Compare against baseline (50us target)
  - Show delta and percentage change
  - Color-code: green (better), amber (within 10%), red (regression)
- **Status**: Live regression tracking
- **Test**: Check Verify page "Latency Regression" section

## Dependencies
- All required dependencies already in requirements.txt:
  - `websockets` for Gateway communication

## Testing Checklist
- [x] Python syntax validation (no errors)
- [ ] Create user endpoint works
- [ ] WAL files shown in viewer (including subdirectories)
- [ ] WAL dump button works
- [ ] Order submission connects to Gateway
- [ ] Latency tracking displays percentiles
- [ ] Scenario switching restarts processes
- [ ] Latency regression shows vs baseline

## Database Note
Database connection uses: `postgresql://postgres:postgres@10.0.2.1:5432/rsx_dev`
This is configured via PG_URL in server.py (line 34-37).

## Implementation Notes

### Order Flow
1. User submits order via form → `/api/orders/test`
2. Server constructs WebSocket message (NewOrder)
3. Connects to Gateway at ws://localhost:8080 with x-user-id header
4. Sends JSON order, waits for response (2s timeout)
5. Parses OrderAccepted/OrderFailed
6. Records latency and status
7. Displays result to user

### Latency Tracking
- Stores last 1000 order latencies
- Calculates percentiles on demand
- Used in two places:
  - Risk page: overall system latency (p50/p95/p99/max)
  - Verify page: regression vs 50us baseline
  - Orders page: per-order latency in table

### WAL File Structure
```
tmp/wal/
  mark/
    100/
      100_active.wal
  pengu/
    10/
      10_active.wal
```
scan_wal_files() now handles this nested structure.
