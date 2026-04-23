# Bucket 1 findings (specs 1-12)

## 1-architecture.md

- §System Overview — **match** (8-process diagram matches actual)
- §Crate Map — **drift** (lists 12 crates including `rsx-playground`; Cargo.toml has 11 Rust crates; rsx-playground is Python, rsx-webui is TS)
- §Order Lifecycle — **match**
- §Hot Path — **bloat** (struct pseudocode for OrderSlot/Slab/CompressionMap, latency tables, bench numbers)
- §Persistence and Recovery — **match**
- §Key Design Decisions — **match** (pg_advisory_lock, SIGTERM=crash, SPSC rings confirmed)
- §Spec Index — **drift** (references old filenames like TILES.md, ORDERBOOK.md)

**Status recommendation**: shipped
**Notable action items**:
- Remove rsx-webui/rsx-playground from Crate Map or label them non-Rust projects
- Update §Spec Index to use current `specs/2/N-name.md` paths
- Move hot-path struct pseudocode and latency tables to code comments or bench files

## 2-archive.md

- §Purpose — **unshipped** (archive client-side fallback documented in DXS §10.5 as "not yet implemented")
- §Deployment — **unshipped** (no archive process/binary exists)
- §API (WAL/TCP) — **unshipped** (no archive TCP server)
- §Recovery Lookup Order — **unshipped**
- §File Layout — **match** (recorder output format matches; slight naming inconsistency: spec uses `first_seq_last_seq`, DXS §8 uses date-based)
- §Gap Handling — **unshipped**
- §Notes — **match**

**Status recommendation**: draft
**Notable action items**:
- Change status from `shipped` to `draft` — archive replay server and consumer fallback are unimplemented
- Resolve file layout naming conflict between 2-archive.md and DXS §8

## 3-cli.md

- §Purpose — **match**
- §wal-dump / dump — **match**
- §Record Types (14 total) — **drift** (FILL field naming: spec says `user_id`, code prints `taker`/`maker`; CAUGHT_UP: spec says `symbol_id`, code prints `stream_id`)
- §Output Formats — **match**
- §Proposed Improvements / Filtering — **match** (all filters shipped)
- §Proposed Improvements / Stats / Follow / Human-Readable — **match** (all shipped)
- §Implementation Plan — **bloat** (fully shipped, stale)
- §Acceptance Criteria — **bloat** (all shipped)

**Status recommendation**: partial
**Notable action items**:
- Remove §Proposed Improvements and §Implementation Plan — all shipped
- Fix drift: FILL field naming and CAUGHT_UP field
- Move acceptance criteria to test file or delete

## 4-cmp.md

- §1 Design — **match** (WAL bytes = disk bytes = wire bytes)
- §2 Wire Format — **match** (WalHeader, CmpRecord trait with seq@byte-0)
- §3 Transport: CMP/UDP — **drift** (CmpSender struct field names stale relative to code)
- §3 Control Messages — **match** (StatusMessage, Nak, CmpHeartbeat at 0x10/0x11/0x12)
- §3 Flow Control — **match**
- §3 Gap Detection — **match**
- §3 Sender/Receiver pseudocode — **bloat**
- §4 WAL Replication over TCP — **match** (DxsReplayService with rustls TLS)
- §4 Config env vars — **match**
- §5 Protocol Patterns — **match**
- §6 Known Pitfalls — **match**
- §7 Comparison — **match**
- §8 Implementation — **bloat** (struct defs; wrong names: `WalReplicationServer` vs actual `DxsReplayService`; `WalReplicationClient` vs `DxsConsumer`)
- §9 Performance Targets — **bloat**

**Status recommendation**: shipped
**Notable action items**:
- Remove/consolidate struct pseudocode in §3 and §8
- Fix struct names: `WalReplicationServer` → `DxsReplayService`, `WalReplicationClient` → `DxsConsumer`

## 5-codepaths.md

- §1 New Order — **match**
- §1 Tests — **drift** (`rsx-matching/tests/fanout_test.rs` doesn't exist)
- §2-11 all codepaths — **match** (all files and tests verified)

**Status recommendation**: shipped
**Notable action items**:
- Fix §1 test file reference: `fanout_test.rs` doesn't exist; fanout tested via `lifecycle_test.rs` and `order_processing_test.rs`

## 6-consistency.md

- §1 Fan-Out: CMP/UDP — **match**
- §2 Ordering Guarantees — **match**
- §3 Backpressure — **match** (OVERLOADED, ME stall, CMP UDP drop)
- §4 Positions & Risk — **drift** (references `PERSISTENCE.md` which doesn't exist)
- §5 Crash Behavior — **match**
- §6 Deferred — **match**
- §Drain Loop Pseudocode — **bloat**
- §Key Invariants — **match**
- §Verification — **bloat**

**Status recommendation**: shipped
**Notable action items**:
- Fix §4 dead link: point to 8-database.md or WAL spec
- Remove §Drain Loop Pseudocode (stale, redundant)
- Trim §Verification to test file refs

## 7-dashboard.md

- §1 Purpose — **unshipped** (no support dashboard exists)
- §2-8 — **unshipped** (nothing implemented; no `/v1/api/support` routes)

**Status recommendation**: draft
**Notable action items**:
- Change `status: shipped` to `status: draft`
- Consider merging with 12-health-dashboard.md to avoid fragmentation

## 8-database.md

- §Recommendation — **match**
- §PostgreSQL — **match** (pg_advisory_lock + synchronous_commit)
- §Redis — **match** (not used)
- §Practical Pattern / Async Persistence — **match** (write-behind, rtrb + 10ms flush)
- §Backpressure Rule — **match** (100ms lag threshold)
- §Are You Rolling Your Own Database? — **bloat** (Q&A artifact)
- §Critique of the Claims — **bloat** (historical design critique)
- §Open Inputs — **bloat** (pre-decision discussion)
- §Five Key Points — **bloat** (Postgres vs RocksDB rationale)

**Status recommendation**: reference
**Notable action items**:
- Remove §Are You Rolling Your Own Database, §Critique, §Open Inputs, §Five Key Points
- Keep implementation notes (write-behind, backpressure, recovery)

## 9-deploy.md

- §Multi-Server Topology — **unshipped** [STUB]
- §Single-Machine Dev Topology — **drift** (references `run.py`, actual is `start`)
- §Configuration (component tables) — **match** (env vars confirmed)
- §Security / Postgres HA / Core Pinning / Monitoring / Rolling Upgrades / Backup / Capacity — **unshipped** [STUB]
- §CMP/UDP Buffer Sizing — **match**
- §Health Endpoints — **drift** (spec shows response fields; actual unverified)
- §Process Supervision — **unshipped** (no systemd units)
- §Log Rotation — **match**
- §Disk Layout — **match**

**Status recommendation**: partial
**Notable action items**:
- Fix §Single-Machine Dev Topology: `run.py` → `start`
- Move [STUB] sections to `unshipped` with tracking notes
- Remove capacity planning table or label as unvalidated estimates

## 10-dxs.md

- §1 WAL Record Format — **drift** (lists 11 types as "v1"; code has 14 data + 4 control; missing: MARK_PRICE, ORDER_REQUEST, ORDER_RESPONSE, CANCEL_REQUEST, ORDER_FAILED, LIQUIDATION, REPLAY_REQUEST)
- §1 Payload Layouts — **bloat** (7 `#[repr(C)]` struct defs)
- §2 File Layout — **match**
- §3 WalWriter — **match** (struct fields correct, some omitted: `last_seq`, `archive_dir`, `listeners`, `flush_stalled`)
- §3 WalWriter pseudocode — **bloat**
- §4 WalReader — **match** (concept + WalFileInfo)
- §4 WalReader pseudocode — **bloat**
- §5 Replay Server — **drift** (calls it `DxsReplay server`; actual `DxsReplayService`)
- §6 Consumer — **drift** (stored callback; actual passes per `run_once`; `producer_addr` is `String` not `SocketAddr`; §10.13 says "client.rs not yet implemented" but client.rs is 572 lines)
- §7 Transport — **match**
- §8 Recorder Pattern — **drift** ("Three recorder instances" but table has four; `recorder.rs` listed in rsx-dxs/ but lives in rsx-recorder/)
- §9 DXS Replaces Existing Specs — **match**
- §10 WAL Replay Edge Cases — **match** (edge cases real; line refs like `wal.rs:393-404` stale)
- §10 Line number references — **bloat**
- §11 Performance Targets — **bloat**
- §12 File Organization — **drift** (`recorder.rs` wrong location)

**Status recommendation**: partial
**Notable action items**:
- Update §1 record type list (14+ types)
- Remove struct pseudocode (§3, §4, §6)
- Fix §6: DxsConsumer callback model, producer_addr type, remove stale "not yet implemented" note
- Fix §8: recorder in rsx-recorder/, fix count (3 vs 4)
- Remove line number citations in §10

## 11-gateway.md

- §Responsibilities — **match**
- §Protocol / Backpressure / Connection Lifecycle — **match**
- §Rate Limits — **match** (10/user, 100/IP, 1000/instance)
- §Limits — **match**
- §Config — **match**
- §REST API — **match** (`/health` + `/v1/*`)
- §Notes — **match**
- §Post-MVP deferred items — **drift** (REST `/v1/*` already shipped: `rest.rs` + `rest_contract_test.rs`)

**Status recommendation**: shipped
**Notable action items**:
- Update/remove §Post-MVP: REST API is shipped

## 12-health-dashboard.md

- §1-6 all sections — **unshipped** (no health dashboard exists; no routes; no RBAC; no alert state machine)

**Status recommendation**: draft
**Notable action items**:
- Change `status: shipped` to `status: draft`
- Consider merging with 7-dashboard.md
