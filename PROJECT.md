# RSX Exchange Implementation

## Goal
Ship the RSX perpetuals exchange from spec to working implementation with all tests passing and edge cases documented.

## Stack
- **Language**: Rust
- **Runtime**: Native binaries per component (Gateway, Risk, ME per symbol, Marketdata, Recorder, Mark)
- **Networking**: monoio (io_uring) for critical path, CMP/UDP for live orders, TCP for WAL replication
- **Storage**: WAL (DXS) for event sourcing, fixed-record format
- **IPC**: SPSC rings (rtrb) for intra-process communication

## IO Surfaces
- **Gateway**: WebSocket ingress (client orders), CMP/UDP egress to risk/ME
- **Marketdata**: WebSocket egress (L2/BBO/trades to clients)
- **Risk/ME**: CMP/UDP for order flow, WAL replication over TCP
- **Mark**: External price feed ingress, CMP broadcast of mark prices
- **Recorder**: DXS consumer, archival storage

## Architecture
See specs/v1/TILES.md, ARCHITECTURE.md, CMP.md for detailed component design.

## Constraints
- <50μs end-to-end Gateway→ME→Gateway latency
- <500ns matching engine match latency
- Zero heap allocation on hot path
- Fixed-point i64 arithmetic (no floats)
- Debug builds default (faster compile)
- Tests: unit <5s, e2e ~30s, integration 1-5min

## Success Criteria
1. **All tests pass**: `make test`, `make e2e`, `make integration`, `make wal`
2. **Spec coverage complete**: Track in PROGRESS.md (currently 85-100% per crate)
3. **Edge cases documented**: All invariants, error paths, and boundary conditions covered in specs
4. **No regressions**: CRITIQUE.md items remain resolved (all 36 items already closed)
5. **Build health**: `cargo check` passes on every change

## Tracking
- **PROGRESS.md**: Implementation audit (% complete per crate, missing features)
- **CRITIQUE.md**: Design issues and resolutions (uppercase per CLAUDE.md)
- **Specs**: specs/v1/*.md define requirements, TESTING-*.md define test coverage

## Workflow
1. Audit current state via PROGRESS.md
2. Identify gaps in spec coverage
3. Implement missing features per spec
4. Run `cargo check` frequently (~every 50 lines)
5. Write tests before marking features complete
6. Update PROGRESS.md after each milestone
7. Document edge cases in relevant specs
8. Iterate until all tests pass and coverage = 100%
