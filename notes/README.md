# notes/

Reference notes on systems topics that come up while building this exchange.
One file per concept, kept short. For researched findings with benchmarks and
external measurements, see `facts/`.

| File | Topic |
|------|-------|
| [align.md](align.md) | `#[repr(C, align(64))]` — why and when |
| [arena.md](arena.md) | Slab/bump allocators — O(1) alloc on hot paths |
| [hotcold.md](hotcold.md) | Hot/cold field splitting — cache line layout |
| [smrb.md](smrb.md) | SPSC ring buffers — intra-process IPC without locks |
| [uds.md](uds.md) | Unix domain sockets vs shared memory |
| [pq.md](pq.md) | `pq` — jq-equivalent for Parquet files |
