# Getting Started

## Prerequisites

- Rust toolchain (`rustup`)
- Python 3.11+ with `uv`
- Node 18+ (for Playwright tests)

## Build

```
cargo build
```

## Run

```
cd rsx-playground && uv run server.py
```

Open [http://localhost:49171](http://localhost:49171).

## Processes

The playground starts and manages these RSX processes:

| Process | Port | Role |
|---------|------|------|
| gateway | 8088 | WebSocket ingress, order routing |
| marketdata | 8081 | L2/BBO/trade streaming |
| risk-0 | — | Margin validation, positions |
| me-\<symbol\> | — | Matching engine (one per symbol) |
| mark | — | Mark price aggregation |
| recorder | — | WAL archival |
| market-maker | — | Automated liquidity bot |

Start all via the Control tab or `POST /api/processes/all/start?confirm=yes`.

## API

- `/healthz` — server + process health
- `/api/status` — full status with maker key
- `/api/maker/status` — market maker levels and PID
- `/api/book/<depth>` — live orderbook snapshot

## Tests

```
cd rsx-playground && bun install
bunx playwright test
```

## Progress

Task progress is tracked in `.ship/tasks.json`.
Regenerate `PROGRESS.md` from source:

```
python3 scripts/task-report.py
```
