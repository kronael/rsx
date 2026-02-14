# RSX Playground Documentation

The RSX Playground is a web-based development dashboard for exploring and testing the RSX exchange system. It provides real-time monitoring, process control, order submission, and fault injection capabilities.

## Documentation Contents

- [Tabs Guide](tabs.md) - Detailed guide for each of the 10 tabs
- [Scenarios](scenarios.md) - Available test scenarios (minimal/duo/full/stress)
- [API Reference](api.md) - HTTP endpoints for process control and orders
- [Troubleshooting](troubleshooting.md) - Common issues and solutions

## Quick Start

```bash
cd rsx-playground
uv run server.py
```

Open http://localhost:49171 in your browser.

## What is the Playground?

The playground is a development tool for:

1. **Process Management** - Start/stop RSX processes (Gateway, Risk, ME, Marketdata, Mark, Recorder)
2. **Order Testing** - Submit test orders via WebSocket, watch fills in real-time
3. **Monitoring** - View system health, processes, logs, metrics, WAL events
4. **Fault Injection** - Kill processes, test recovery, validate correctness
5. **Verification** - Run 10 invariant checks, reconciliation, latency regression

## Documentation vs System Documentation

**This documentation (rsx-playground/docs/):** How to USE the playground UI

- Tab functionality and features
- Scenario selection (minimal/duo/full/stress)
- API endpoints for testing
- Troubleshooting UI/process issues

**System documentation (../specs/v1/, ../architecture/):** How the RSX SYSTEM works

- Architecture (CMP/UDP, WAL, tiles, SPSC rings)
- Orderbook algorithm (Slab, CompressionMap, price-time priority)
- Risk engine logic (margin, positions, funding, liquidation)
- Matching engine semantics (GTC/IOC/FOK, post-only, reduce-only)
- Consistency guarantees (exactly-once delivery, FIFO, etc.)

**To view system documentation:**

```bash
cd ..
./scripts/serve-docs.sh
```

Then visit http://localhost:8001 for full RSX documentation.

Or browse:
- [../README.md](../README.md) - Project overview
- [../specs/v1/ARCHITECTURE.md](../specs/v1/ARCHITECTURE.md) - System architecture
- [../PROGRESS.md](../PROGRESS.md) - Per-crate implementation status

## Tabs Overview

- **Overview** - System health, process status, key metrics
- **Topology** - Process graph, core affinity, CMP flows
- **Book** - Order book ladder, live fills, trade aggregation
- **Risk** - Position heatmap, margin, funding, liquidations
- **WAL** - Write-ahead log status, lag, timeline
- **Logs** - Unified log viewer with smart search
- **Control** - Start/stop processes, scenario selection
- **Faults** - Fault injection (kill processes)
- **Verify** - Run invariant checks, reconciliation
- **Orders** - Submit test orders, view recent orders

## Typical Workflow

1. Start the playground: `uv run server.py`
2. Visit http://localhost:49171
3. Select a scenario (minimal, duo, full, stress)
4. Click "Build & Start All" on the Overview tab
5. Submit test orders on the Orders tab
6. Monitor fills and logs
7. Inject faults on the Faults tab to test recovery
8. Run verification checks on the Verify tab

## Requirements

- Python 3.14+
- uv (Python package manager)
- Rust/Cargo (for building RSX binaries)
- PostgreSQL (optional, for risk/gateway features)

## See Also

- [Full RSX Documentation](http://localhost:8001) (run `../scripts/serve-docs.sh`)
- [Project README](../README.md)
- [CLAUDE.md](../CLAUDE.md) - Development conventions
