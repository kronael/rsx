# API Reference

REST and HTMX partial endpoints exposed by the RSX playground server.

## REST API

### Orders

| Method | Path | Description |
|--------|------|-------------|
| POST | /api/order | Submit a new order |
| DELETE | /api/order/{oid} | Cancel an order |
| GET | /api/orders | List recent orders |

### Book

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/book/{depth} | Orderbook snapshot |
| GET | /api/bbo | Best bid/offer |

### Risk

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/risk/users/{uid} | User risk state |
| POST | /api/risk/users/{uid}/freeze | Freeze user |
| POST | /api/risk/users/{uid}/unfreeze | Unfreeze user |
| POST | /api/risk/liquidate | Trigger liquidation |

### Status

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/status | System health |
| GET | /api/maker/status | Market maker health |

## HTMX Partials

All `/x/*` endpoints return HTML fragments for HTMX polling.

| Path | Description |
|------|-------------|
| /x/position-heatmap | Net position per symbol |
| /x/margin-ladder | Recent fills and notional |
| /x/funding | BBO-derived funding rate |
| /x/risk-latency | Order latency percentiles |
| /x/book-stats | Live BBO table |
| /x/live-fills | Recent trade fills |
| /x/processes | Process health grid |
| /x/health | Health summary |
| /x/wal-status | WAL writer status |
| /x/logs | Recent log lines |
