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

Process graph, core affinity map, and CMP flow diagram.
Shows which CPU cores each process is pinned to.

## Book

Live orderbook ladder with bid/ask levels.
Updates on each market data event.

## Risk

Position heatmap, margin ladder, and funding rate panel.
Reads from WAL fills and BBO history.

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

## Trade

React-based trading UI served from the rsx-webui build.
Full order blotter and live market data.
