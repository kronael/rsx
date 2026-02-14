# Playground Tabs

The RSX Playground UI is organized into 10 tabs, each focused on a specific aspect of the system.

## Overview

**Purpose:** System health at a glance

**Features:**
- Health score (process uptime + DB connectivity)
- Process table with CPU/memory/uptime
- Scenario selector (minimal/duo/full/stress)
- "Build & Start All" and "Stop All" buttons
- Key metrics (active orders, positions, WAL files)
- Ring backpressure (SPSC queue fullness)
- WAL status per process
- Log tail (last 20 lines)
- Invariants status

**Auto-refresh:** 2s for most cards, 5s for invariants

**Use case:** Quick check before submitting orders or after injecting faults

## Topology

**Purpose:** Understand the process graph and network topology

**Features:**
- Process graph (ASCII art showing CMP/UDP and WAL/TCP flows)
- Core affinity map (which process pinned to which core)
- CMP connection status (sent/recv/NAK/drop counters)
- Process list

**Auto-refresh:** 2s for CMP flows, 5s for core affinity

**Use case:** Verify all processes started, check CMP connectivity, validate core pinning

## Book

**Purpose:** Real-time orderbook visualization

**Features:**
- Symbol selector (PENGU/SOL/BTC/ETH)
- Orderbook ladder (price levels with qty on bid/ask)
- Best bid/ask spread
- Book stats (total depth, number of levels)
- Live fills (recent trades with price/qty/side)
- Trade aggregation (1min buckets: volume, count, price range)

**Auto-refresh:** 1s for book and fills, 2s for stats

**Use case:** Watch orderbook after submitting limit orders, verify fills happen

## Risk

**Purpose:** Margin, positions, funding, liquidations

**Features:**
- Position heatmap (users x symbols, color-coded by size)
- Margin ladder (users sorted by margin ratio)
- Funding payments (per symbol, per interval)
- Liquidation queue (pending liquidations)
- Risk check latency (p50/p95/p99 histograms)
- User lookup (by user_id: balance, positions, margin, frozen status)
- User actions: create, deposit, freeze, unfreeze, liquidate

**Auto-refresh:** 2s for heatmap/margin/funding/liquidations, 5s for latency

**Use case:** Check margin before submitting large orders, trigger liquidation, monitor funding

## WAL

**Purpose:** Write-ahead log health and event stream

**Features:**
- Per-process WAL state (tip seq, active file, flush lag)
- Lag dashboard (producer seq - consumer seq per stream)
- Rotation / tip health (flush lag, rotation status)
- Timeline (last 100 WAL events, filterable by type)
- WAL files (list all active/rotated files with size/modified)
- Verify button (run WAL corruption checks)
- Dump JSON button (export WAL to JSON for analysis)

**Auto-refresh:** 1s for lag, 2s for state/rotation/timeline, 5s for files

**Use case:** Verify WAL replication lag, check for stalls, dump events for debugging

## Logs

**Purpose:** Unified log viewer with filtering

**Features:**
- Smart search (natural language: "gateway error order" or plain text)
- Quick filters (gateway, risk, matching, errors only, warnings only)
- Process filter dropdown (all/gateway/risk/matching/marketdata/mark/recorder)
- Level filter dropdown (all/error/warn/info/debug)
- Text search box
- Click-to-expand for long log lines
- Copy button in modal
- Error aggregation (group by pattern, show count)
- Auth failures (last 10 failed JWT/rate-limit events)
- Keyboard shortcuts: `/` to focus search, `Ctrl+L` to clear, `Escape` to close modal

**Auto-refresh:** 2s for logs (last 1000 lines), 5s for error agg

**Use case:** Debug order rejections, find errors after fault injection, monitor process startup

## Control

**Purpose:** Process lifecycle management

**Features:**
- Scenario selector (minimal/duo/full/stress-low/stress-high/stress-ultra)
- Switch scenario button
- Current scenario display
- Per-process control grid (start/stop/restart/kill buttons)
- Resource usage bars (CPU/memory per process)
- Notes (common commands: `./start full`, `./start -c`, `./start --reset-db`)

**Auto-refresh:** 2s for control grid, 5s for resource usage

**Use case:** Start/stop individual processes, switch between scenarios, restart after crashes

## Faults

**Purpose:** Fault injection for testing recovery

**Features:**
- Per-process fault grid (stop/kill buttons)
- Restart button (appears when process stopped)
- Recovery notes (how to test network faults, WAL corruption)

**Auto-refresh:** 2s for fault grid

**Use case:** Kill ME to test WAL replay, kill gateway to test reconnection, kill risk to test failover

## Verify

**Purpose:** Run correctness checks

**Features:**
- Invariants (10 system correctness rules: fills precede ORDER_DONE, exactly-one completion, FIFO, etc.)
- Run All Checks button
- Last run timestamp per check
- Reconciliation (frozen margin vs computed, shadow book vs ME book, mark price vs index)
- Latency regression (GW->ME->GW p99, ME match p99 vs baseline)
- E2E test link (run `cargo test` directly)

**Auto-refresh:** 5s for invariants/reconciliation/latency

**Use case:** Verify system correctness after fault injection, detect regressions, ensure consistency

## Orders

**Purpose:** Submit test orders and trace lifecycle

**Features:**
- Order submission form (symbol/side/type/price/qty/TIF/user_id/reduce_only/post_only)
- Quick actions: Batch (10), Random (5), Stress (100), Invalid
- Order result display (oid, status, fills)
- Order lifecycle trace (enter oid, see all WAL events: ACCEPTED, FILL, DONE)
- Recent orders (last 50 with cancel button)
- Stale orders (>1 hour unfilled)

**Auto-refresh:** 2s for recent orders, 10s for stale orders

**Use case:** Submit limit/market orders, trace order lifecycle, cancel orders, stress test

## Navigation

**Tabs are always visible at the top.** Click any tab to switch. The "Docs" tab (if present) opens the full RSX documentation in a new window at http://localhost:8001.

## Tips

- Use keyboard shortcuts in Logs tab: `/` to search, `Ctrl+L` to clear
- Click log lines to expand and copy
- Use "Batch" and "Random" buttons in Orders tab for quick testing
- Monitor Overview tab after clicking "Build & Start All" (30-60s startup)
- Check Logs tab for errors if processes fail to start
- Use Stress scenarios for load testing (stress-ultra = 500 orders/sec x 10s)
