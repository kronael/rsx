# Documentation Quick Reference

Fast lookup for documentation locations. See DOCUMENTATION.md for
full index.

---

## I Need To...

### Understand the System

| Task | Document |
|------|----------|
| Get project overview | README.md |
| Understand architecture | ARCHITECTURE.md, specs/v1/ARCHITECTURE.md |
| Learn about threads/tiles | specs/v1/TILES.md |
| Understand networking | specs/v1/NETWORK.md |
| Learn about WAL/recovery | specs/v1/WAL.md, specs/v1/DXS.md |
| See implementation status | PROGRESS.md |
| Find remaining work | TODO.md |

### Work on a Component

| Component | Spec | Testing Spec | Crate Docs |
|-----------|------|--------------|------------|
| Orderbook | ORDERBOOK.md | TESTING-BOOK.md | rsx-book/ |
| Matching Engine | MATCHING.md | TESTING-MATCHING.md | rsx-matching/ |
| Risk Engine | RISK.md | TESTING-RISK.md | rsx-risk/ |
| Gateway | GATEWAY.md | TESTING-GATEWAY.md | rsx-gateway/ |
| Market Data | MARKETDATA.md | TESTING-MARKETDATA.md | rsx-marketdata/ |
| Mark Price | MARK.md | TESTING-MARK.md | rsx-mark/ |
| Liquidator | LIQUIDATOR.md | TESTING-LIQUIDATOR.md | (in rsx-risk) |
| WAL/DXS | DXS.md, WAL.md | TESTING-DXS.md | rsx-dxs/ |

All specs in `specs/v1/`, crate docs in `<crate>/README.md`.

### Write Tests

| Test Type | Document | Location |
|-----------|----------|----------|
| Overall strategy | specs/v1/TESTING.md | - |
| Component tests | specs/v1/TESTING-*.md | tests/ in each crate |
| Edge cases | VALIDATION-EDGE-CASES.md | - |
| Test validation | TEST-VALIDATION-REPORT.md | - |

### Debug Production Issues

| Issue | Document |
|-------|----------|
| System crashed | RECOVERY-RUNBOOK.md, CRASH-SCENARIOS.md |
| Need to monitor | MONITORING.md, specs/v1/TELEMETRY.md |
| Understand guarantees | GUARANTEES.md, specs/v1/CONSISTENCY.md |
| Deploy changes | specs/v1/DEPLOY.md |

### Understand Protocols

| Protocol | Document |
|----------|----------|
| CMP (UDP transport) | specs/v1/CMP.md |
| WebSocket messages | specs/v1/WEBPROTO.md |
| RPC protocol | specs/v1/RPC.md |
| REST API | specs/v1/REST.md |
| Internal messages | specs/v1/MESSAGES.md |

### Work on Dashboard/UI

| Dashboard | Document |
|-----------|----------|
| Playground (dev tool) | specs/v1/PLAYGROUND-DASHBOARD.md |
| Playground screens | specs/playground/SCREENS.md |
| Playground API | specs/playground/SPEC.md |
| Health monitoring | specs/v1/HEALTH-DASHBOARD.md |
| Risk monitoring | specs/v1/RISK-DASHBOARD.md |
| Operations | specs/v1/MANAGEMENT-DASHBOARD.md |
| General dashboard | specs/v1/DASHBOARD.md |
| Frontend design | FRONTEND.md, SCREENS.md |

### Learn Implementation Patterns

| Pattern | Document |
|---------|----------|
| SPSC ring buffers | notes/SMRB.md, specs/v1/TESTING-SMRB.md |
| Memory alignment | notes/ALIGN.md |
| Arena allocators | notes/ARENA.md |
| Hot/cold paths | notes/HOTCOLD.md |
| Priority queues | notes/PQ.md |
| Unix domain sockets | notes/UDS.md |

### Read Blog Posts

| Topic | Document |
|-------|----------|
| All posts | blog/README.md (index) |
| Design philosophy | blog/01-design-philosophy.md |
| Matching engine | blog/02-matching-engine.md |
| Risk engine | blog/03-risk-engine.md |
| WAL and recovery | blog/04-wal-and-recovery.md |
| Testing approach | blog/06-test-suite-archaeology.md |
| Performance | blog/18-100ns-matching.md |

### Follow Development Conventions

| Convention | Document |
|------------|----------|
| Code style | CLAUDE.md |
| Commit format | CLAUDE.md |
| Testing strategy | CLAUDE.md, specs/v1/TESTING.md |
| Build commands | CLAUDE.md, Makefile |

---

## File Naming Patterns

- `UPPERCASE.md` - Important project docs (root level)
- `specs/v1/*.md` - Specifications (current version)
- `specs/v2/*.md` - Future specifications
- `notes/*.md` - Implementation notes
- `blog/*.md` - Blog posts
- `<crate>/README.md` - Crate user documentation
- `<crate>/ARCHITECTURE.md` - Crate internals
- `tests/*_test.rs` - Unit tests
- `tests/*.rs` - Integration tests

---

## Common Paths

```
/home/onvos/sandbox/rsx/
├── README.md                    # Start here
├── CLAUDE.md                    # Development conventions
├── ARCHITECTURE.md              # System overview
├── PROGRESS.md                  # Implementation status
├── TODO.md                      # Remaining work
├── DOCUMENTATION.md             # Full documentation index
├── DOCS-MANIFEST.md             # Documentation tracking
├── DOCS-CLEANUP-PLAN.md         # Cleanup plan
├── specs/
│   ├── v1/                      # Current specifications
│   │   ├── ARCHITECTURE.md
│   │   ├── ORDERBOOK.md
│   │   ├── MATCHING.md
│   │   ├── RISK.md
│   │   ├── GATEWAY.md
│   │   ├── MARKETDATA.md
│   │   ├── MARK.md
│   │   ├── LIQUIDATOR.md
│   │   ├── DXS.md
│   │   ├── WAL.md
│   │   ├── CMP.md
│   │   ├── TILES.md
│   │   ├── NETWORK.md
│   │   ├── TESTING-*.md         # Testing specs
│   │   └── ...
│   ├── v2/                      # Future specifications
│   └── playground/              # Playground detailed specs
├── blog/                        # Blog posts
├── notes/                       # Implementation notes
├── rsx-book/                    # Orderbook crate
├── rsx-matching/                # Matching engine crate
├── rsx-risk/                    # Risk engine crate
├── rsx-dxs/                     # WAL/DXS crate
├── rsx-gateway/                 # Gateway crate
├── rsx-marketdata/              # Market data crate
├── rsx-mark/                    # Mark price crate
├── rsx-recorder/                # Recorder crate
├── rsx-cli/                     # CLI tool crate
└── rsx-types/                   # Shared types crate
```

---

## Documentation Dependencies

Core dependency chain:

```
README.md
  → ARCHITECTURE.md
      → specs/v1/ARCHITECTURE.md
          → specs/v1/TILES.md (threads)
          → specs/v1/NETWORK.md (networking)
              → specs/v1/CMP.md (transport)
              → specs/v1/GATEWAY.md (ingress)
          → specs/v1/ORDERBOOK.md (data structure)
              → specs/v1/MATCHING.md (logic)
                  → specs/v1/RISK.md (margin)
                      → specs/v1/LIQUIDATOR.md
                  → specs/v1/MARKETDATA.md (broadcast)
                      → specs/v1/MARK.md (pricing)
          → specs/v1/DXS.md (replication)
              → specs/v1/WAL.md (persistence)
```

---

## Quick Links by Role

### New Developer

1. README.md
2. ARCHITECTURE.md
3. CLAUDE.md
4. specs/v1/TILES.md
5. Pick a component spec

### Component Developer

1. specs/v1/<COMPONENT>.md
2. specs/v1/TESTING-<COMPONENT>.md
3. rsx-<component>/README.md
4. PROGRESS.md (status)

### System Architect

1. ARCHITECTURE.md
2. specs/v1/ARCHITECTURE.md
3. specs/v1/TILES.md
4. specs/v1/NETWORK.md
5. specs/v1/CONSISTENCY.md

### Operations Engineer

1. MONITORING.md
2. RECOVERY-RUNBOOK.md
3. specs/v1/DEPLOY.md
4. specs/v1/TELEMETRY.md
5. CRASH-SCENARIOS.md

### QA/Test Engineer

1. specs/v1/TESTING.md
2. specs/v1/TESTING-*.md
3. VALIDATION-EDGE-CASES.md
4. TEST-VALIDATION-REPORT.md

---

## Last Updated

2026-02-13
