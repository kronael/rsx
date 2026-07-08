# RSX Playground

RSX is a derivatives exchange playground for local development and testing.

## Overview

- **Gateway** — WebSocket ingress, order routing
- **Risk** — margin validation, position tracking
- **ME** — matching engine (one per symbol)
- **Marketdata** — L2/BBO/trade streaming
- **Mark** — mark price aggregation
- **Recorder** — WAL archival

## Usage

```
cd rsx-playground && uv run server.py
```

Then open [http://localhost:49171](http://localhost:49171).

## Pages

| Page | Description |
|------|-------------|
| Overview | Process health and key metrics |
| Topology | Process graph and core affinity |
| Book | Live orderbook ladder |
| Risk | Margin and position state |
| WAL | Write-ahead log status |
| Logs | Tail log output |
| Control | Start/stop processes |
| Faults | Fault injection |
| Verify | Invariant checks |
| Orders | Order entry and management |
| Stress | Load test runner |
| Trade | React trading UI |
