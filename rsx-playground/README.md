# RSX Playground

Web-based development dashboard for the RSX exchange. Real-time monitoring, process control, order submission, and fault injection.

## Quick Start

```bash
cd rsx-playground
uv run server.py
```

Open http://localhost:49171, click "Start All". Processes auto-shutdown after 5min idle.

## What is the Playground?

A UI tool for developers to:

- Start/stop RSX processes (Gateway, Risk, ME, Marketdata, Mark, Recorder)
- Submit test orders via WebSocket
- Monitor system health (processes, WAL, logs, metrics)
- View orderbooks and fills in real-time
- Inspect WAL events and replication lag
- Inject faults (kill processes, test recovery)
- Verify correctness (10 invariants, reconciliation)

## Documentation

**Playground Documentation:** See [docs/README.md](docs/README.md)

Includes:
- [Tabs Guide](docs/tabs.md) - What each tab does
- [Scenarios](docs/scenarios.md) - minimal/duo/full/stress configs
- [API Reference](docs/api.md) - HTTP endpoints
- [Troubleshooting](docs/troubleshooting.md) - Common issues

**Project Documentation:** See [../README.md](../README.md) and [../specs/v1/ARCHITECTURE.md](../specs/v1/ARCHITECTURE.md)

Or run `../scripts/serve-docs.sh` and visit http://localhost:8001 for full RSX documentation.

## Typical Workflow

1. Start playground: `uv run server.py`
2. Visit http://localhost:49171
3. Click "Start All" (30-60s build time)
4. Wait for processes to start (green dots in table)
5. Orders tab: Submit test orders
6. Book tab: Watch fills happen
7. Logs tab: Monitor errors
8. Faults tab: Kill a process, watch recovery
9. Verify tab: Run invariant checks

Note: Processes auto-shutdown after 5min idle to save resources.

## Requirements

- Python 3.14+
- uv (Python package manager)
- Rust/Cargo (for building RSX binaries)
- PostgreSQL (optional, for risk/gateway features)

Install uv:
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

## Scenarios

- **minimal (1Z):** 1 symbol, no replication, 5 processes, 20s startup
- **duo (2Z):** 2 symbols, no replication, 6 processes, 25s startup
- **full (3):** 3 symbols, mark price, 8 processes, 35s startup
- **stress-low:** 10 orders/sec x 60s
- **stress-high:** 100 orders/sec x 60s
- **stress-ultra:** 500 orders/sec x 10s

See [docs/scenarios.md](docs/scenarios.md) for details.

## Tabs

- **Overview:** System health, processes, metrics, logs
- **Topology:** Process graph, core affinity, CMP flows
- **Book:** Orderbook ladder, fills, trades
- **Risk:** Positions, margin, funding, liquidations
- **WAL:** Write-ahead log status, lag, events
- **Logs:** Unified log viewer with smart search
- **Control:** Start/stop processes, scenario switching
- **Faults:** Fault injection (kill processes)
- **Verify:** Invariant checks, reconciliation
- **Orders:** Submit test orders, view recent orders

See [docs/tabs.md](docs/tabs.md) for tab details.

## API

All endpoints documented in [docs/api.md](docs/api.md).

Examples:

```bash
# Start all processes
curl -X POST http://localhost:49171/api/processes/all/start \
  -H "Content-Type: application/json" \
  -d '{"scenario": "minimal"}'

# Stop all processes
curl -X POST http://localhost:49171/api/processes/all/stop

# Submit test order
curl -X POST http://localhost:49171/api/orders/test \
  -H "Content-Type: application/json" \
  -d '{
    "symbol_id": 1,
    "side": "buy",
    "order_type": "limit",
    "price": "50000",
    "qty": "1.0",
    "tif": "GTC",
    "user_id": 1
  }'

# Get recent orders
curl http://localhost:49171/x/recent-orders

# Get process list
curl http://localhost:49171/x/processes
```

## Testing

```bash
# Unit tests (Python)
pytest tests/

# E2E tests (Playwright)
cd tests
npm install
npx playwright test

# Or use Make targets
make test        # Python unit tests
make e2e         # Playwright E2E tests
make smoke       # Full smoke test
```

## Troubleshooting

See [docs/troubleshooting.md](docs/troubleshooting.md) for common issues.

Quick fixes:

**Processes won't start:**
```bash
# Check logs
tail -f tmp/unified.log

# Rebuild binaries
cd ..
cargo build --workspace
cd rsx-playground
```

**Orders not submitting:**
```bash
# Check Gateway running
curl http://localhost:8080/health

# Restart Gateway
curl -X POST http://localhost:49171/api/processes/gateway/restart
```

**UI not loading:**
- Hard refresh: `Ctrl+Shift+R`
- Check browser console (F12)
- Check playground terminal for errors

## Architecture

The playground is a Python web server (server.py) that:

1. Manages RSX process lifecycle (start/stop/restart)
2. Parses process logs and WAL files
3. Provides HTTP/HTMX endpoints for UI
4. Submits test orders via WebSocket to Gateway
5. Queries Postgres for risk/user data

**Not a production component.** Development/testing tool only.

For RSX system architecture, see [../specs/v1/ARCHITECTURE.md](../specs/v1/ARCHITECTURE.md).

## Files

```
rsx-playground/
├── server.py           # Main web server (ASGI/HTMX)
├── pages.py            # HTML generation (inline Tailwind)
├── stress_client.py    # Load generator for stress scenarios
├── requirements.txt    # Python dependencies
├── pyproject.toml      # Project metadata
├── docs/               # Playground documentation
│   ├── README.md       # Overview (you are here)
│   ├── tabs.md         # Tab-by-tab guide
│   ├── scenarios.md    # Scenario descriptions
│   ├── api.md          # HTTP API reference
│   └── troubleshooting.md  # Common issues
├── tests/              # Playwright E2E tests
│   ├── test_*.py       # Python tests
│   └── play_*.spec.ts  # Playwright tests
└── tmp/                # Runtime files (logs, WAL, PIDs)
    ├── wal/            # WAL files per process
    ├── unified.log     # Aggregated logs
    └── pids/           # Process PID files
```

## Separation: Playground vs Project Docs

**Playground docs (this directory):** How to USE the playground UI
- Tab functionality
- Scenario selection
- API endpoints
- Troubleshooting UI issues

**Project docs (../specs/v1/, ../architecture/):** How the SYSTEM works
- Architecture (CMP/UDP, WAL, tiles)
- Orderbook algorithm
- Risk engine logic
- Matching engine semantics
- Consistency guarantees

The playground is a tool to EXPLORE the system. Its docs are a "user manual" for the tool, not the system itself.

## See Also

- [Full RSX Documentation](http://localhost:8001) (run `../scripts/serve-docs.sh`)
- [Project README](../README.md)
- [PROGRESS.md](../PROGRESS.md) - Per-crate status
- [CLAUDE.md](../CLAUDE.md) - Development conventions
- [GUARANTEES.md](../GUARANTEES.md) - Consistency guarantees
