# Round 4 Report — root docs hygiene + cross-cut consistency

Master at start: `9903839`. Master at end: `26e5753`.

## Commits (6)

```
1a3b673 [docs] features: 4h WAL retention (not 48h); protocol.rs -> records.rs
bae28a6 [progress] WAL: V1 byte 0 (V0 retired); WalWriter::prepare+append_framed
eb444cc [blog] WalWriter API + 4h retention (not 48h)
983216b [spec] testing-matching: drop flow-control M7; event buf is 65_536 heap-boxed
91e4b9f [arch] rsx-cast: drop StatusMessage from records list; 4h retention default
26e5753 [docs] drop broken .ship/ refs (pruned audit dirs)
```

## Per-file edits

- **FEATURES.md** — 48h → 4h retention; `protocol.rs` → `records.rs`.
- **PROGRESS.md** — V1 at byte 0 (V0 retired); `WalWriter::prepare + append_framed`; dropped broken `.ship/12-SHOWCASE-HONEST/` + `.ship/13-A16Z-FIXES/` refs.
- **BLOG.md** — same WalWriter API fix; 48h → 4h; broken `.ship/15-PLAYGROUND-AUDIT/FINDINGS.md` ref dropped.
- **specs/2/41-testing-matching.md** — M7 flow-control test description rewritten (flow-control removed in 87b223e); event-buf 10k → 65_536 heap-boxed.
- **rsx-cast/ARCHITECTURE.md** — StatusMessage dropped from records list; retention numbers fixed and labelled "design target, not enforced".

Clean (no edits needed): README.md, ONEPAGER.md, MONITORING.md, TESTING.md.

## Validations

- All cited file paths resolve except 3 `.ship/` dirs pruned on close-out — fixed.
- All cited commit hashes verified via `git rev-parse`.

## Items flagged for CTO / CEO

1. **WAL retention documented but not implemented.** All docs say 4h; `rsx-cast/src/wal.rs` has 64MB rotation but no time-based GC. Either wire it or label the doc.
2. **Perf tables (rsx-cast/ARCH, PROGRESS, BLOG)** date from v0.2.0; re-run after Round 1+2 transport cuts to confirm RTT + send-body numbers hold.
3. **Test count drift** across MEMORY (887+) / ONEPAGER (887) / BLOG (883) / PROGRESS (878) / TESTING (878) / FEATURES (878). Pick one truth source.
4. **`specs/2/50-wedge.md` still exists** despite MEMORY claim of "deleted 2026-05-24". Delete the spec OR update MEMORY.

## LOC delta

+21 / -19 (+2 net; mostly rewrites). `cargo check --workspace` green throughout.
