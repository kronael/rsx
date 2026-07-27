# bench/go-vs-rust — doc conventions

Standalone Go-vs-Rust concurrency benchmark. NOT part of the cargo
workspace (own `rust/Cargo.toml` with an empty `[workspace]`) and not in the
Go module of anything else.

## Which file answers which question

| File | Question |
|---|---|
| `README.md` | Which language wins, when, by how much — the 2-minute answer |
| `notes/design.md` | Why these two cases, why the folklore cases were rejected, fairness rules |
| `facts/*.md` | Dated raw numbers from actual runs (source of truth for every number quoted) |
| `go/`, `rust/` | The twin implementations (mirrored 1:1, checksum-verified) |
| `run.py` | Builds, warms up, runs 5 reps, verifies checksums, prints the table |

## Keeper sections — don't regress

- README's "When this flips" and "Honest caveats" sections: the caveats are
  the deliverable, not decoration. Never shorten them away.
- README quotes numbers only from a dated `facts/*.md`; facts cite the exact
  command + versions. Never inline a number without a facts entry behind it.
- notes/design.md's "Folklore cases we measured" (P1/P2): evidence we did
  not strawman Rust. Keep the probes runnable.
- The tuned rows (Go `GOGC=1000`, tokio `block_in_place`) stay in every
  results table — fairness depends on showing the loser's documented fix.
- The cross-language checksum gate in `run.py`: identical-work proof. Any
  workload change must keep Go and Rust checksums equal.

## Update checklist

1. Change a workload → change BOTH twins → `make lint` → `./run.py --quick`
   (checksums must match).
2. New numbers worth keeping → new dated `facts/YYYYMMDD_*.md` → update the
   numbers README quotes → cite the facts file.
3. `make fmt && make lint` before committing (gofmt + vet, rustfmt + clippy).
