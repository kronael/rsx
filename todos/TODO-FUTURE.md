# TODO: Future Analysis and Spec Gaps

Items from GUARANTEES.md, LEFTOSPEC.md, and spec files that are
deferred or need analysis before implementation.

## GUARANTEES.md Section 13: Future Analysis (4 items)

### [ ] 13.1 Quantified Stress Test Targets
- Source: GUARANTEES.md:1073
- Run actual stress tests to validate:
  - Matching engine: 1M fills/sec with 10ms WAL flush
  - Risk: 1M fills/sec with 10ms Postgres flush
  - DXS replay: 100K fills/sec to 10 concurrent consumers
  - Postgres: 100K position updates/sec in batches

### [ ] 13.2 Multi-Datacenter Replication
- Source: GUARANTEES.md:1081
- Specify guarantees for geo-distributed deployments:
  - Cross-DC latency impact on WAL flush
  - Cross-DC replica lag bounds
  - Partition tolerance across DC link failure

### [ ] 13.3 Snapshot Frequency vs Replay Time
- Source: GUARANTEES.md:1091
- Analyze tradeoff:
  - More frequent snapshots = faster recovery, higher I/O
  - Less frequent snapshots = slower recovery, lower I/O

### [ ] 13.4 WAL Retention vs Disk Usage
- Source: GUARANTEES.md:1100
- Analyze:
  - Worst-case disk usage for 10min retention
  - Replay time if consumer lags >10min

## LEFTOSPEC.md Remaining Items (2)

### [ ] Binance feed reconnect details (~10 lines)
- Reconnect backoff: 1s, 2s, 4s, 8s, max 30s
- Staleness threshold: 10s no update = stale
- Stale behavior: use last known, log warning

### [ ] Modify order (deferred to v2)
- v1: cancel + re-insert (explicit)
- v2: atomic modify-in-place

## LIQUIDATOR.md

### [ ] Symbol halt on liquidation failure
- Source: specs/v1/LIQUIDATOR.md:347
- When liquidation fails repeatedly, halt symbol trading
- Implementation TODO in spec
