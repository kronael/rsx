# rsx-book comparisons

Comparison material is split into two evidence classes:

- **Same-harness contenders** run through
  `benches/compare_all_bench.rs` on one machine and one operation stream.
- **Cited contenders** use numbers published by another project on different
  hardware and workloads. They are not treated as direct measurements.

## Capability matrix

| contender | insert and cancel | reduce | best read | order-level FIFO match |
|---|---|---|---|---|
| rsx-book | yes | yes | yes | yes |
| local `BTreeMap` baseline | yes | yes | yes | yes |
| hftbacktest `BTreeMarketDepth` | yes | yes | yes | no — aggregated L2 |
| hftbacktest `HashMapMarketDepth` | yes | yes | yes | no — aggregated L2 |
| `lob-rs` | yes | no amend API | no public API | yes |
| `orderbook` by inv2004 | evaluation fails; see its comparison note | — | — | — |
| `orderbook-rs` by joaquinbejar | cited only | — | — | — |

The local comparison harness currently records Criterion median estimates but
does not collect p99 for every contender. Exact latency tables are therefore
omitted until the harness produces p50 and p99 from the same run.

## Honest trade

A plain `BTreeMap` is the simpler container and has less bookkeeping in the
aggregate-level churn probe. That probe inserts a price entry, changes its
aggregate quantity, and removes it. It does not model the full cost of
per-order identity, FIFO chains, direct cancel by handle, compressed level
storage, maintained BBO, or next-level occupancy lookup.

`rsx-book` carries those structures on every operation. The comparison is
useful only when the workload and capability difference stay visible. It does
not show that every order-book workload benefits from this design.

## Fairness

| contender | same box | same harness | same operation stream | evidence |
|---|---|---|---|---|
| local `BTreeMap` baseline | yes | yes | yes | direct, tail data pending |
| hftbacktest variants | yes | yes | level operations only | direct, tail data pending |
| `lob-rs` | yes | yes | supported operations only | direct, tail data pending |
| `orderbook` by inv2004 | no | no | no | evaluation note |
| `orderbook-rs` by joaquinbejar | no | no | no | evaluation note |
| itch-order-book | no | no | no | cited |
| exchange-core | no | no | no | cited |
| Liquibook | no | no | no | cited |

## Detail

- [naive-btree.md](naive-btree.md)
- [hftbacktest.md](hftbacktest.md)
- [lob.md](lob.md)
- [orderbook-inv2004.md](orderbook-inv2004.md)
- [orderbook-rs.md](orderbook-rs.md)
- [cross-language-cited.md](cross-language-cited.md)

Dated RSX benchmark runs live in the repository-level
[`reports/`](../../reports/) directory.
