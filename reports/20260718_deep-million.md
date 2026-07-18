# Million-order book drain — 2026-07-18

Commit `d59bfd1` plus the uncommitted evidence harness was run with:

```sh
make perf PERF_FILTER=deep_million
```

Release mode seeded 1,000,000 unit sell orders over 1,000 price levels, then
drained all of them in 34 aggressive IOC cascades capped at 30,000 fills. The
two runs took 108 ms and 175 ms inside the harness; the latest peaked at
174,240 KiB RSS. They verified 1,000,000 unique FIFO maker fills, exact filled
quantity, no resting best bid
or ask, all one million slab slots returned to the free list, and immediate
free-slot reuse.

The same run verified that one 32,768-fill aggressor panics with
`INVARIANT: ME event buffer overflow (MAX_EVENTS=65536)` rather than truncating
events. A safe cascade is at most 32,767 fully consumed makers: each emits a
fill and maker completion, followed by taker completion and at most one BBO.
Resident depth and per-order cascade capacity are separate limits; this does
not claim one order can sweep one million makers.

Host: Linux 6.1.0-43-amd64, AMD Ryzen 9 5950X VM with 8 online CPUs and 7.8 GiB
RAM. This is an in-process release build on a shared development host, not a
bare-metal latency result. The observed 108–175 ms harness range excludes Cargo
startup and compilation. Latest output:

```text
oversize_boundary=panic_verified fills=32768 MAX_EVENTS=65536
deep_million orders=1000000 cascades=34 max_fills_per_cascade=30000 elapsed_ms=175 peak_rss_kb=174240
```
