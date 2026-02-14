# Playground Scenarios

The playground supports multiple scenarios for different testing needs. Select a scenario on the Overview or Control tab, then click "Build & Start All".

## Minimal (1Z)

**Processes:**
- Gateway
- Risk (1 shard)
- ME-BTCUSD (matching engine for BTC-PERP)
- Marketdata
- Recorder

**Use case:** Basic testing with a single symbol. Fastest startup (~20s).

**Configuration:**
- 1 symbol (BTC-PERP, symbol_id=1)
- No replication
- No mark price feed
- 1 risk shard

**When to use:**
- Quick smoke tests
- Order submission testing
- WAL inspection
- Gateway/Risk/ME integration testing

## Duo (2Z)

**Processes:**
- Gateway
- Risk (1 shard)
- ME-BTCUSD (symbol_id=1)
- ME-ETHUSD (symbol_id=2)
- Marketdata
- Recorder

**Use case:** Multi-symbol testing without full complexity.

**Configuration:**
- 2 symbols (BTC-PERP, ETH-PERP)
- No replication
- No mark price feed
- 1 risk shard

**When to use:**
- Cross-symbol testing
- Multiple orderbooks
- Position across multiple symbols
- Funding across symbols

## Full (3 symbols)

**Processes:**
- Gateway
- Risk (1 shard)
- ME-BTCUSD (symbol_id=1)
- ME-ETHUSD (symbol_id=2)
- ME-SOLUSD (symbol_id=3)
- Marketdata
- Mark (mark price aggregator)
- Recorder

**Use case:** Full system with mark price feed.

**Configuration:**
- 3 symbols (BTC-PERP, ETH-PERP, SOL-PERP)
- Mark price aggregator (Binance/Coinbase feeds)
- 1 risk shard
- Full CMP mesh (Gateway, Risk, ME, Marketdata, Mark, Recorder)

**When to use:**
- Production-like testing
- Mark price integration
- Full CMP topology validation
- Liquidation with mark price
- Insurance fund testing

## Stress-Low (10 orders/sec x 60s)

**Processes:** Same as Full

**Load generator:**
- 10 orders/sec (limit and market mixed)
- 60 seconds duration
- 3 symbols
- 5 users

**Use case:** Sustained load testing, verify no leaks or stalls.

**Monitoring:**
- Check ring backpressure (Overview tab)
- Watch WAL lag (WAL tab)
- Monitor CPU/memory (Control tab)
- Verify latencies (Verify tab)

**When to use:**
- Pre-production load testing
- Detect memory leaks
- Validate WAL rotation
- Check for backpressure

## Stress-High (100 orders/sec x 60s)

**Processes:** Same as Full

**Load generator:**
- 100 orders/sec (limit and market mixed)
- 60 seconds duration
- 3 symbols
- 10 users

**Use case:** Heavy load testing, find bottlenecks.

**Monitoring:**
- Ring backpressure should stay <50%
- WAL lag should stay <100ms
- CPU should stay <80% per core
- Latencies: GW->ME->GW p99 <100us

**When to use:**
- Stress testing
- Latency regression detection
- Find backpressure points
- Validate rate limiting

## Stress-Ultra (500 orders/sec x 10s)

**Processes:** Same as Full

**Load generator:**
- 500 orders/sec (limit and market mixed)
- 10 seconds duration (short burst)
- 3 symbols
- 20 users

**Use case:** Extreme burst testing, push system limits.

**Expected behavior:**
- Ring backpressure 60-80%
- WAL lag spikes to 200-500ms
- Some orders rejected (rate limit)
- Recovery within 1s after burst ends

**When to use:**
- Find breaking points
- Validate backpressure handling
- Test rate limiter
- Verify recovery after burst

## Custom Scenarios

To create a custom scenario:

1. Edit `server.py` and add a new scenario in `START_SCENARIOS`
2. Define which processes to start
3. Add load generator config (optional)
4. Restart the playground: `uv run server.py`

Example:

```python
START_SCENARIOS["custom"] = {
    "procs": ["gateway", "risk", "me_btcusd", "marketdata"],
    "symbols": [1],
    "load": {"rate": 20, "duration": 30, "users": 3},
}
```

## Switching Scenarios

Two ways to switch:

1. **Overview tab:** Select scenario from dropdown, click "Build & Start All"
2. **Control tab:** Use scenario selector, click "Switch Scenario"

**Note:** Switching scenarios stops all running processes, cleans WAL files, and rebuilds binaries if needed.

## Scenario Comparison

| Scenario | Processes | Symbols | Mark Price | Load | Startup Time | Use Case |
|----------|-----------|---------|------------|------|--------------|----------|
| Minimal | 5 | 1 | No | None | 20s | Quick smoke |
| Duo | 6 | 2 | No | None | 25s | Multi-symbol |
| Full | 8 | 3 | Yes | None | 35s | Production-like |
| Stress-Low | 8 | 3 | Yes | 10/s x 60s | 40s | Sustained load |
| Stress-High | 8 | 3 | Yes | 100/s x 60s | 40s | Heavy load |
| Stress-Ultra | 8 | 3 | Yes | 500/s x 10s | 40s | Burst testing |

## Tips

- Start with Minimal for quick testing
- Use Full for integration testing
- Use Stress scenarios only when monitoring tabs are open
- Monitor Logs tab during stress tests for errors
- Run "make clean" before switching scenarios if binaries fail to start
- Check PROGRESS.md for per-crate status if processes crash
