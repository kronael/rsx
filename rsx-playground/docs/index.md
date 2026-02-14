# RSX Playground Documentation Index

Quick navigation for all playground documentation.

## Getting Started

- [README](README.md) - Overview and quick start

## User Guides

- [Tabs Guide](tabs.md) - Detailed guide for each of the 10 tabs
- [Scenarios Guide](scenarios.md) - Available test scenarios (minimal/duo/full/stress)
- [Troubleshooting](troubleshooting.md) - Common issues and solutions

## Developer Reference

- [API Reference](api.md) - HTTP endpoints for process control, orders, queries

## Quick Links

**Playground UI:** http://localhost:49171

**System Documentation:** http://localhost:8001 (run `../scripts/serve-docs.sh`)

## Documentation Structure

```
rsx-playground/docs/          # Playground-specific docs (this directory)
├── README.md                 # Overview, what is the playground
├── tabs.md                   # UI guide: what each tab does
├── scenarios.md              # Test scenarios (minimal/duo/full/stress)
├── api.md                    # HTTP API reference
└── troubleshooting.md        # Common issues

../specs/v1/                  # RSX system specs
├── ARCHITECTURE.md           # System architecture
├── ORDERBOOK.md              # Orderbook algorithm
├── RISK.md                   # Risk engine logic
├── DXS.md                    # WAL format and streaming
├── CMP.md                    # CMP/UDP protocol
└── ...

../architecture/              # Component architecture docs
├── gateway.md                # Gateway internals
├── matching.md               # Matching engine internals
├── risk.md                   # Risk engine internals
└── ...
```

## Playground vs System Docs

**Playground docs (this directory):**
- How to USE the playground UI
- Tab functionality
- Scenario selection
- API endpoints
- Troubleshooting UI issues

**System docs (../specs/v1/, ../architecture/):**
- How the RSX SYSTEM works
- Architecture (CMP/UDP, WAL, tiles)
- Orderbook algorithm
- Risk engine logic
- Consistency guarantees

## Navigation Tips

- **From Playground UI:** Click "Playground Guide" in footer
- **From Playground UI to System Docs:** Click "Full Documentation" in footer
- **From Terminal:** `cd rsx-playground && cat docs/README.md`
- **View System Docs:** `cd .. && ./scripts/serve-docs.sh`

## Contributing to Docs

When adding playground features:

1. Update [tabs.md](tabs.md) if adding/changing tabs
2. Update [api.md](api.md) if adding API endpoints
3. Update [scenarios.md](scenarios.md) if adding scenarios
4. Update [troubleshooting.md](troubleshooting.md) for new error cases
5. Keep docs focused on HOW TO USE the playground, not HOW THE SYSTEM WORKS

For system architecture changes, update `../specs/v1/` instead.
