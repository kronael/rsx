# Monitoring Guide

RSX uses structured logging for all metrics and observability. Logs are emitted as JSON lines that can be shipped to any log aggregation system.

## Log Format

All components emit structured JSON logs:

```json
{
  "ts": "2026-02-13T10:34:26.123456Z",
  "level": "INFO",
  "component": "risk",
  "shard_id": 0,
  "event": "position_update",
  "user_id": 1234,
  "symbol_id": 1,
  "position": 1000000,
  "margin_used": 50000
}
```

## Key Metrics

### Gateway

```
ws_connections_active       gauge
ws_auth_failures            counter
rate_limit_exceeded         counter
circuit_breaker_open        gauge
order_submit_latency_us     histogram
```

### Risk Engine

```
margin_checks_passed        counter
margin_checks_failed        counter
position_updates            counter
funding_payments            counter
liquidations_triggered      counter
margin_check_latency_us     histogram
```

### Matching Engine

```
orders_matched              counter
orders_rejected             counter
book_depth_bids             gauge
book_depth_asks             gauge
match_latency_ns            histogram
```

## Alerting

See [specs/v1/TELEMETRY.md](../specs/v1/TELEMETRY.md) for complete metrics catalog.
