# Stress Test Feature - Quick Reference

## Overview

The playground now has a dedicated **Stress Testing** tab with:
- Interactive launcher to run stress tests at any rate/duration
- Automatic HTML report generation with charts and metrics
- Historical report browser with clickable timestamps
- Pass/Fail assessment based on production criteria

## How to Use

### 1. Navigate to Stress Tab

Open http://localhost:49171/stress

### 2. Configure and Launch Test

- **Rate**: Orders per second (10 - 10,000)
- **Duration**: Test duration in seconds (1 - 600)
- Click "Run Stress Test"

The test will run asynchronously and save results automatically.

### 3. View Reports

Click on any timestamp in the "Historical Reports" table to view:
- Summary metrics (target vs actual rate, elapsed time)
- Results breakdown (submitted, accepted, rejected, errors)
- Latency distribution with percentiles (min, p50, p95, p99, max)
- Pass/Fail assessment against production criteria

## Report Criteria

A stress test **PASSES** if all criteria are met:

| Metric | Threshold | Purpose |
|--------|-----------|---------|
| Accept Rate | ≥95% | Risk engine validation |
| p99 Latency | <10ms | Round-trip performance |
| Error Rate | <1% | System stability |

Color coding:
- 🟢 Green: Good performance
- 🟡 Amber: Acceptable but watch
- 🔴 Red: Failed threshold

## API Endpoints

### Run Stress Test
```bash
POST /api/stress/run?rate=1000&duration=60
```

Returns JSON with full results and saves HTML report.

### List All Reports
```bash
GET /api/stress/reports
```

Returns JSON array of all historical reports.

### Get Specific Report
```bash
GET /api/stress/reports/{timestamp}
```

Returns JSON data for a specific report.

### View Report HTML
```
GET /stress/{timestamp}
```

Displays full HTML report with charts.

## File Locations

Reports saved to: `/home/onvos/sandbox/rsx/tmp/stress-reports/`

Format: `stress-YYYYMMDD-HHMMSS.json`

Example: `stress-20260213-214530.json`

## Architecture

```
┌─ Stress Tab (/stress)
│
├─ Launcher Form
│  └─ POST /api/stress/run
│     ├─ Calls stress_client.py
│     ├─ Runs WebSocket workers
│     ├─ Collects metrics
│     └─ Saves to tmp/stress-reports/
│
├─ Reports List
│  └─ GET /x/stress-reports-list (HTMX)
│     └─ Scans tmp/stress-reports/*.json
│        └─ Displays table with links
│
└─ Individual Report (/stress/{id})
   └─ GET /stress/{id}
      ├─ Loads JSON data
      ├─ Generates HTML with charts
      └─ Shows pass/fail assessment
```

## Example Reports

See `STRESS-TEST-EXAMPLE.md` for:
- 1,000 orders/sec sustained (60s)
- 10,000 orders/sec sustained (10 min)
- Ramp-up testing (100 → 15k orders/sec)
- Breaking point analysis

## Integration with Stress Client

The stress launcher uses `stress_client.py`:

```python
from stress_client import run_stress_test, StressConfig

config = StressConfig(
    gateway_url="ws://localhost:8080",
    rate=1000,  # orders/sec
    duration=60,  # seconds
    users=10,
    connections=10,
)

results = await run_stress_test(config)
```

Returns:
```json
{
  "config": {"target_rate": 1000, "duration": 60, ...},
  "metrics": {
    "submitted": 60000,
    "accepted": 57850,
    "rejected": 1950,
    "errors": 200,
    "actual_rate": 998.2,
    "accept_rate": 96.4,
    "elapsed_sec": 60.11
  },
  "latency_us": {
    "p50": 245,
    "p95": 890,
    "p99": 1450,
    "min": 120,
    "max": 8240
  }
}
```

## Dependencies

- `websockets`: WebSocket client for Gateway communication
- `asyncio`: Async/await support for concurrent workers
- Already in requirements.txt

## Next Steps

1. Run baseline test: 100 orders/sec × 60s
2. Ramp up gradually: 500, 1k, 2.5k, 5k, 10k
3. Find breaking point
4. Optimize bottlenecks
5. Repeat until target performance achieved

## Success Indicators

✅ Sustained 10k orders/sec for 10 minutes
✅ p99 latency <1ms (1000µs)
✅ >95% acceptance rate
✅ <1% error rate
✅ No process crashes
✅ WAL lag <50ms throughout

When all indicators green → Production ready for 10k orders/sec workload.
