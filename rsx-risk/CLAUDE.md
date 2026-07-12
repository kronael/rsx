# CLAUDE.md — rsx-risk

Local to `rsx-risk/`. Inherits the repo-root `../CLAUDE.md`. This file
pins the crate's **doc conventions and the invariants that must not
regress**. The full topology + rationale lives in the `doc-topology`
skill; exemplars to mirror are rsx-cast and rsx-book.

## Doc topology (which file answers which question)

Follow the `doc-topology` skill. Table only the docs that actually exist
here:

| File | The one question | Notes |
|---|---|---|
| `README.md` | what / why / how-to-run | what the tile does, `Running` (env vars: shard id/count, core, WAL dir, cast addrs), an `Internal architecture` summary, and a `How to read this crate` index |
| `ARCHITECTURE.md` | how it's built | risk framed as the **pre-trade gate** before the ME; hot loop, margin math, fill/funding/liquidation, the persist ring → tokio sidecar (PG write-behind + lease), main/replica failover |
| `notes/*.md` | why each IPC / comms choice | `spsc.md` (rtrb SPSC rings over channels/mutex/shmem), `uds.md` (why not Unix domain sockets or shared memory cross-process). One file per decision, indexed by `notes/README.md` |

No `compare/` or `facts/` dir here. Dated numbers live in root `reports/`;
the crate's Criterion harness is under `benches/`.

## Keeper sections — do NOT regress

Load-bearing; a "simplification" that drops one is a bug, not a cleanup:

- **The `How to read this crate` index in `README.md`.** It is the
  told-not-implied topology; keep the what/how/why pointers.
- **The pre-trade-gate framing in `ARCHITECTURE.md`.** Risk is the gate
  that keeps one over-leveraged trader from making the exchange
  insolvent — don't let a rewrite reduce it to "it checks margin".
- **The hot-path / off-path split.** Docs must keep it explicit: the
  pinned loop is std non-blocking UDP busy-spin (no async runtime) and a
  tokio sidecar owns PG write-behind + lease renewal, with handoff over
  the persist SPSC ring. Never collapse this to "it's async".
- **`notes/spsc.md` and `notes/uds.md` rationale.** The
  rings-vs-channels-vs-mutex and UDS/shmem-rejected reasoning is the
  answer to "why not just use a channel / a socket" — a change that drops
  it re-opens a settled decision.

## When you touch this crate

- New IPC/comms decision → a `notes/` file (Problem → Fix → Cost) + a row
  in `notes/README.md`.
- New measured number → land it in root `reports/YYYYMMDD_*.md`, then
  quote it with the bench name + caveat; never inline a raw number.
