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

## Terminal

Embedded `rsx-term` session. The Playground opens a local PTY,
runs the Go terminal against `ws://127.0.0.1:8088` and
`ws://127.0.0.1:8180`, and streams it through xterm.js.
