# UI Tabs

The RSX playground is organized into tabs for each concern.

## Walkthrough

Interactive system walkthrough and landing page. 9 sections
covering architecture, order lifecycle, matching engine,
risk, WAL/transport, market data, mark price, benchmarks,
and a "Try It" button to start the exchange. Each section
has a TL;DR summary and expandable details.

## Overview

Process health, key metrics, and system invariants.
Refreshes every 2 seconds via HTMX polling.

## Topology

Process graph, core affinity map, and casting flow diagram.
Shows which CPU cores each process is pinned to.

## Book

Live orderbook ladder with bid/ask levels.
Updates on each market data event.

## Risk

Position heatmap, margin ladder, and funding rate panel.
Reads from WAL fills and BBO history.

## Gateway

The gateways: per-gateway state, open connections, orders forwarded to
risk, rejects, pending set (used / cap) and circuit-breaker state, all
read from each gateway's `/metrics`; rejects by reason under their
spec 18 names; and the duplicate client order id demo, which sends one
IOC order twice with the same client order id over the real gateway and
shows the gateway answering the repeat with `DUPLICATE_ORDER_ID`.

## Matching

The matching engines: per-ME counters (orders, publishes, rejects by
reason, drops, dedup window), order entry with GTC, IOC, FOK, Post Only
and Reduce Only, recent orders with a Cancel button that cancels a
resting order at the engine through the gateway, the duplicate order id
demo (a scratch engine, since the gateway never repeats an id), the
recovery card (WAL tip, checkpoint, config version, and a crash-and-recover
button that shows them before and after), and the engine-made rejects and
cancels from the ME WAL with their wire reason.

## Users

Admin-only (`role=admin`, checked live against `rsx-auth` on every
load). List users with their role and collateral, create a user,
deposit, and change another user's role (proxied to `rsx-auth`'s own
RBAC-gated `/roles/*` endpoints -- reason + ticket ID required, audit
row per change).

## Config

Admin-only (`role=admin`, checked live). The active row per symbol from
`symbol_config_schedule`: tick, lot, minimum order, margin rates, max
leverage, fees, funding interval and version. The minimum order is shown
in USD, converted from risk's raw `price x qty` unit with that symbol's
own decimals (`min_notional / 10^(price_decimals + qty_decimals)`). The
form schedules a new version, effective in N seconds; the minimum is
entered in USD (blank keeps the current one), rejected unless positive,
and written as the raw integer. A new version is a new row; the active
row is never updated. Risk reads it at boot and on CONFIG_APPLIED.

## WAL

Write-ahead log status: file rotation, tip sequence, lag.
Links to WAL detail and timeline views.

## Logs

Tail of combined log output from all processes.
Filter by process or log level.

## Control

Start/stop individual processes. Run scenarios.
Inject faults and trigger liquidations.

## Faults

Fault injection panel: simulate network partitions,
process crashes, and WAL corruption.

## Verify

Invariant check results: book consistency, position sums,
funding zero-sum, and WAL replay correctness.

## Orders

Order entry form and recent order list.
Submit limit/market orders and cancel pending ones.

## Stress

Load test runner with configurable rate and duration.
Shows throughput, latency histogram, and error rate.
