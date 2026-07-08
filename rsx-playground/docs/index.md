# RSX Playground Documentation

## Getting Started

- [README](./README) — Overview and setup
- [API Reference](./api) — REST and HTMX endpoints
- [UI Tabs](./tabs) — What each tab does
- [Scenarios](./scenarios) — Pre-built test scenarios
- [Troubleshooting](./troubleshooting) — Common fixes

## Architecture

RSX is a derivatives exchange with separate processes
communicating over casting/UDP and WAL replication over TCP.

```
Gateway → Risk → ME → Marketdata
              ↘ WAL ↙
           Recorder
           Market Maker (bot)
```

## Quick Start

```
cargo build
cd rsx-playground && uv run server.py
```

Open [http://localhost:49171](http://localhost:49171).

## Project Status

Derived from `.ship/tasks.json` — 199/340 tasks complete (59%).

| Status | Count |
|--------|-------|
| completed | 199 |
| running | 7 |
| pending | 134 |
| failed | 0 |

Run `python3 scripts/task-report.py` to update `PROGRESS.md`.
