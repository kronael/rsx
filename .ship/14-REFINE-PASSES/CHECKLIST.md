# 14-REFINE-PASSES — checklist

Goal: leave the system **minimal, orthogonal, working, and not
overly complicated**. Every round = one concern, one focused
sub-agent, one diff, one commit (or "no action needed" + log
line).

Wisdom anchors (`~/.claude/skills/rs/SKILL.md`):
- NEVER `let _ = call_returning_result()` — propagate via `?`,
  log via `if let Err`, or fail loud
- NEVER bare `.unwrap()` in non-test code — use `.expect("msg")`
- All hot-path code returns Result, never panics
- Single import per line; no nested mods; flat crates
- Crate-per-concern; flat lib.rs re-exports
- `#[repr(C, align(64))]` on hot-path structs
- Fixed-point i64; `Price`/`Qty` newtypes; never floats
- Document lock acquisition order
- Pin hot threads via core_affinity; SPSC rings via rtrb
- Tests in dedicated `tests/` dir (not inline)

Baseline metrics (before pass):
- Workspace: 12 Rust crates, **878 Rust tests pass, 0 fail**
- 93 `let _ =` sites (workspace, non-test)
- 362 `.unwrap()` sites (workspace, non-test)
- `rsx-dxs` production lib has zero `rsx-types` dep
- Bench gate: existing baselines in `bench-baseline.json`

Per-round protocol:
1. Spawn 1 sub-agent with the **brief from this file** for that
   round. The sub returns a < 200-word report plus a unified diff
   (or "no action needed" + the rationale).
2. Apply diff if any; run the gate listed in the round.
3. Commit `[refine] <crate-or-aspect>: <one-line summary>`.
4. Tick the box here.

Batching: **up to 4 rounds in parallel per turn** when buckets
are independent (per-crate hygiene; per-spec audit). Cross-
cutting sweeps run sequentially because they touch many files.

After every batch: `cargo check --workspace --tests --benches`
+ `cargo test --workspace`. Workspace must stay green.

---

## Bucket A — wisdom-violation sweeps (cross-cutting)

These run sequentially; each touches many files.

- [ ] **A1.** `let _ =` sweep across all crates. Classify each:
      (a) legitimate (e.g. `let _ = drop_handle`) — leave;
      (b) drops a Result — convert to `?`, `if let Err`, or
      `.expect(...)`. Gate: `cargo test --workspace`.
- [ ] **A2.** `.unwrap()` sweep across non-test code. Classify
      each: (a) startup fail-fast — replace with `.expect(...)`
      and `// SAFETY: fail-fast at startup` comment;
      (b) hot-path — return Result and propagate via `?`;
      (c) genuinely impossible — `.expect("INVARIANT: ...")` with
      named invariant. Gate: tests.
- [ ] **A3.** `.expect("...")` audit. Each call must either be
      labeled `// SAFETY: fail-fast at startup` or contain a
      named invariant in the message. Gate: tests.
- [ ] **A4.** `panic!()` / `todo!()` / `unimplemented!()` /
      `unreachable!()` audit. Outside of tests, only allowed
      with a comment justifying impossibility. Gate: tests.
- [ ] **A5.** `// TODO` / `// FIXME` / `// HACK` audit. Each
      must either link a tracking task or be deleted. Gate: none.
- [ ] **A6.** Single-import-per-line audit. `use foo::{a, b}` →
      two lines. Gate: `cargo check`.
- [ ] **A7.** Dead code sweep — `#[allow(dead_code)]`, unused
      `pub` re-exports, unused fields. Gate: tests + clippy.
- [ ] **A8.** Comment hygiene: drop "this fixes ...", "removed
      because ...", "TODO see issue #N". Comments explain
      hidden invariants only. Gate: none.

## Bucket B — per-crate hygiene (12 rounds, parallelisable in 4s)

Each round: one sub reads `<crate>/src/`, audits against rs
wisdom, returns a focused diff. Gate: `cargo test -p <crate>`.

- [ ] **B1.** `rsx-types`
- [ ] **B2.** `rsx-dxs` (transport)
- [ ] **B3.** `rsx-messages`
- [ ] **B4.** `rsx-book` (orderbook)
- [ ] **B5.** `rsx-matching` (ME)
- [ ] **B6.** `rsx-risk` (positions, liquidation, replication)
- [ ] **B7.** `rsx-gateway` (WS, JWT, rate limit, REST)
- [ ] **B8.** `rsx-marketdata` (shadow book, L2)
- [ ] **B9.** `rsx-mark` (aggregator)
- [ ] **B10.** `rsx-recorder` (archival)
- [ ] **B11.** `rsx-cli` (WAL dump)
- [ ] **B12.** `rsx-maker` (market maker bot)

## Bucket C — per-spec audit (key specs only, ~12 rounds)

Each round: one sub reads spec + the implementing code, flags
contradictions, suggests minimal fixes. Gate: `cargo check`.

- [ ] **C1.** `specs/2/4-cmp.md` ↔ `rsx-dxs/src/cmp.rs`
- [ ] **C2.** `specs/2/10-dxs.md` ↔ `rsx-dxs/src/{server,client}.rs`
- [ ] **C3.** `specs/2/17-matching.md` + `21-orderbook.md` ↔ `rsx-book` + `rsx-matching`
- [ ] **C4.** `specs/2/28-risk.md` ↔ `rsx-risk/src/`
- [ ] **C5.** `specs/2/16-marketdata.md` ↔ `rsx-marketdata/src/`
- [ ] **C6.** `specs/2/11-gateway.md` + `49-webproto.md` ↔ `rsx-gateway/src/`
- [ ] **C7.** `specs/2/15-mark.md` ↔ `rsx-mark/src/`
- [ ] **C8.** `specs/2/18-messages.md` ↔ `rsx-messages/src/`
- [ ] **C9.** `specs/2/45-tiles.md` ↔ pinning + ring usage across crates
- [ ] **C10.** `specs/2/48-wal.md` ↔ `rsx-dxs/src/wal.rs`
- [ ] **C11.** `specs/2/47-validation-edge-cases.md` ↔ all validators
- [ ] **C12.** `specs/2/6-consistency.md` ↔ Invariants #1–#10 enforcement

## Bucket D — orthogonality + minimisation (8 rounds)

Each round: identify a coupling that shouldn't exist, or a
feature that's overbuilt. Land a contraction.

- [ ] **D1.** `repr(C)` layout invariants — verify size + align
      compile-time asserts on every wire record. Gate: tests.
- [ ] **D2.** CRC coverage — every `from_bytes` path must verify
      CRC before any unsafe cast. Gate: tests.
- [ ] **D3.** Tracing/logging hygiene — uniform `subsystem:
      message` format; no debug-prints; no `println!` in
      non-test code. Gate: clippy.
- [ ] **D4.** Hot-path heap allocs — sweep matching, gateway,
      marketdata for `Vec::new`, `vec!`, `String::new`,
      `Box::new`, `.to_vec()`, `.clone()`, `.to_string()`,
      `format!`. Each must justify or move to startup. Gate:
      tests + bench.
- [ ] **D5.** Re-exports / public surface — every crate's
      `lib.rs` should re-export only what consumers actually
      use (per `cargo tree` consumers). Trim unused re-exports.
- [ ] **D6.** Test-only API leakage — `pub(crate)` instead of
      `pub` for anything only tests consume. Gate: tests.
- [ ] **D7.** Dependency graph — `cargo tree --duplicates` and
      check no two version of any crate; check each `Cargo.toml`
      lists only what's used. Gate: clippy.
- [ ] **D8.** Spec-only-mentioned files / orphan modules —
      anything in `src/` not pulled in via `lib.rs` or `main.rs`
      tree. Delete or wire up.

## Final pass — verification + REPORT

- [ ] **F1.** `cargo test --workspace` green; record passing
      count.
- [ ] **F2.** `cargo clippy --workspace -- -D warnings` (the
      project's `make lint`) — only pre-existing warnings, no
      regressions.
- [ ] **F3.** `cargo bench` smoke — baselines unchanged within
      10% (bench-gate).
- [ ] **F4.** `make e2e` — Playwright + API still green.
- [ ] **F5.** Append `.ship/14-REFINE-PASSES/REPORT.md` —
      one line per round: bucket-id + commit-hash + before/after
      metric.
- [ ] **F6.** Update `PROGRESS.md` if any crate's "Open" column
      changed.

---

## Order of execution

Start with **Bucket A** (highest leverage; the wisdom rules
just landed and they govern the rest). Then **Bucket B**
(per-crate, naturally parallel). Then **Bucket C** (spec
contradictions surface after the code is clean). Then
**Bucket D** (orthogonality is easier to spot when noise is
gone). Finish with **F1–F6**.

Total: **8 + 12 + 12 + 8 + 6 = ~46 rounds.**
