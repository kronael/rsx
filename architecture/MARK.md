# Mark Price Aggregator Architecture

External exchange feeds, median aggregation, CMP to risk.

## Data Flow

```
Binance WS    Coinbase WS    (+ N sources)
+--------+    +----------+   +------+
| Trades |    | Trades   |   | ...  |
+--------+    +----------+   +------+
    |              |              |
    v              v              v
+------------------------------------------+
| Mark Aggregator                          |
|                                          |
| Per-symbol state:                        |
|   sources[]  (last price + timestamp)    |
|   median calculation                     |
|   staleness filtering                    |
|                                          |
| Main loop:                               |
|   1. Drain SPSC rings from sources       |
|   2. Sweep stale (1s interval)           |
|   3. Aggregate -> MarkPriceEvent         |
|   4. WAL append                          |
|   5. CMP send to Risk                    |
|   6. Flush WAL (10ms)                    |
+-----+----+-------------------------------+
      |    |
      v    v
    WAL   CMP/UDP -> Risk
      |
      v
    DxsReplay -> consumers
```

## Aggregation

- Median of non-stale sources per symbol
- Staleness: source older than configured threshold
- `sweep_stale()` runs every 1s
- `aggregate_with_staleness()` produces mark price

## Sources

- BinanceSource: tokio-tungstenite WS
- CoinbaseSource: tokio-tungstenite WS
- Connected via SPSC rings to main aggregator

## CMP Feed to Risk

Mark price sent via CmpSender (RECORD_MARK_PRICE)
to risk engine for margin calculation and
liquidation triggers.

## Specs

- [specs/v1/MARK.md](../specs/v1/MARK.md)
- [specs/v1/TESTING-MARK.md](../specs/v1/TESTING-MARK.md)
