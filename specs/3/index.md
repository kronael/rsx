# Specs — Phase 3

Phase-3 specs: work past the phase-2 exchange — reusable primitives,
scaling, and new markets. Phase 2 is the current architecture
([../2/index.md](../2/index.md)); phase 1 is historical
([../1/index.md](../1/index.md)).

Status legend: **draft** — written, not implemented · **spec** — designed,
awaiting build · **shipped** — in production code.

| # | Spec | Status | Summary |
|---|------|--------|---------|
| 1 | [1-cast-failover-transport.md](1-cast-failover-transport.md) | draft | Cast as a transport-agnostic, failover-capable broker: runtime-free failover coordinator (fence injected) + io_uring/SQPOLL transport decoupling |
