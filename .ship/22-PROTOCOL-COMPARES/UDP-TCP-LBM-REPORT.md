# Protocol Compares — UDP / TCP / LBM

## Deliverables

| Path | Change |
|---|---|
| `rsx-dxs/compare/raw-udp.md` | Polished: added UDP wire-format table, explicit guarantees table (UDP vs CMP), RFC 768 + udp(7) citations. |
| `rsx-dxs/compare/tcp.md` | Expanded from 52 LOC to a full doc: wire model, guarantees table, hot-vs-cold-path rationale, sources (RFC 9293, RFC 2018, RFC 896, BBR paper). Updated expected-numbers table with measured value. |
| `rsx-dxs/compare/lbm.md` | Polished: explicit "no bench possible" up top, lineage diagram, guarantees table (LBM vs CMP), Informatica + Aeron sources. |
| `rsx-dxs/benches/compare_tcp.rs` | New: Criterion bench, std `TcpListener`/`TcpStream`, `TCP_NODELAY` both ends, nonblocking + spin, persistent connection, partial-recv-correct via `read_exact_spin`. |
| `rsx-dxs/benches/udp_rtt_bench.rs` | No change — already mature; imports + docstring already conform. |
| `rsx-dxs/Cargo.toml` | Added `[[bench]] name = "compare_tcp"`, `harness = false`. No new deps (stdlib only). |
| `rsx-dxs/compare/README.md` | Summary-table TCP row updated with measured values; added `compare_tcp` to run-commands block. |

## Bench result

```
tcp_rtt_loopback_64b    time: [12.279 µs 15.300 µs 18.241 µs]
```

(Smoke run with `--sample-size 10 --measurement-time 2`. P50 ≈ 15 µs.)

Direct comparison against the existing bench suite on the same host:

| Bench | P50 |
|---|---|
| `udp_rtt_bench` (raw UDP) | ~2 µs |
| `cmp_rtt_bench` (rsx-dxs CMP) | ~10 µs |
| `compare_tcp` (std TCP nodelay + spin, new) | ~12–18 µs |
| `compare_quinn` tcp_rtt_nodelay (tokio TCP) | ~100–1 000 µs |

The std-TCP-spin number is ~2× CMP, much closer than the
"~100× penalty" the iggy benchmark suggested — because the
iggy comparison was against tokio TCP through a reactor.
The doc was updated to reflect both regimes honestly: TCP
through a reactor is what production order-flow servers
actually pay; the kernel TCP floor is much faster but still
loses to UDP on the points that matter (head-of-line
blocking, one-stream funnel, no multicast).

## Oracle review

Asked codex to review `compare_tcp.rs` for correctness. Output:

| Finding | Verdict | Action |
|---|---|---|
| `b.iter` swallows EOF / non-WouldBlock errors via `black_box(ok)` — if the echo thread dies mid-bench, Criterion records meaningless near-zero timings. | **Valid.** | Fixed: replaced `black_box(ok)` with `assert!(...)` on both write and read. |
| TCP_NODELAY on both ends? | Pass — confirmed lines 107 + 122. | None. |
| Connection reused, no reconnect in loop? | Pass. | None. |
| read/write drain exact PAYLOAD bytes? | Pass — `read_exact_spin` / `write_all_spin` handle partial I/O. | None. |
| Nonblocking + spin consistent? | Pass. | None. |
| Off-by-one? | None found. | None. |
| Shutdown race? | Not a bench-validity issue — no in-flight RTT remains after last iteration. | None. |

Skipped: none. Took: the `assert!` fix.

Post-fix rebuild + smoke run both clean.

## Notes / open items

- TCP doc no longer claims "~100× penalty" — that figure is
  reactor-specific. The hot-path argument for CMP over TCP
  stands on multiplexing + head-of-line blocking + no
  multicast, not raw RTT.
- LBM cannot be benchmarked; doc states so up top.
- udp_rtt_bench.rs needed no polish — single-import-per-line
  was already in place, docstring was already focused.
