# CLAUDE.md — rsx-matching

Local to `rsx-matching/`; the root `../CLAUDE.md` applies. The matching
engine is the authoritative writer of fills, one process per symbol: orders
from risk, matching against `rsx-book`, the WAL, fan-out to risk and
marketdata. The algorithm is `rsx-book`'s; this crate is the loop around it.
Specs: `specs/2/17-matching.md`, `21-orderbook.md` (ME-owned sections),
`6-consistency.md`, `48-wal.md`; tests `41-testing-matching.md`.

## Rules

- It is a single pinned loop, not a tile: casting I/O, dedup, match, WAL
  append and fan-out run inline on `RSX_ME_CORE_ID`. Call it a loop or a
  process. No external I/O in the loop: the Postgres config poll runs on a
  side-thread and hands results over an rtrb ring; the loop never blocks on
  a database.
- Layout: `main.rs` → `app.rs` (boot, recovery sequence) → `cast_loop.rs`
  (the loop) → `order.rs` / `cancel.rs` (handlers); lib modules
  `checkpoint`, `config`, `dedup`, `recovery`, `wal`, `metrics`, `wire`
  (decoders only). Wire records are `rsx-messages` types; NEVER a local copy
  or a mangled variant of a struct that already exists there.
- Config comes from the applied Postgres config (`symbol_config_schedule`,
  owned by `rsx-risk`); the ME is a reader. An env fallback when no applied
  row exists is a recorded gap, not the design.
- The dedup window is 1 hour, keyed by `(user_id, order_id)`. The ME never
  sees a client order id: the gateway answers a same-cid resend itself, and
  a new cid is a new order id.
- Every rejection and cancel reason is a wire code (`FailureReason`,
  `FillOrKill = 13` included), never free text.
- Checkpoints are Avro (`checkpoint-v1.avsc`) with a developer-maintained
  schema version and the WAL seq inside. The book hands out a snapshot of
  pointers on the loop; serialization and the tmp + fsync + rename dance run
  on the checkpoint worker. Correctness and a self-describing format are the
  point here, not speed.
- Recovery: load the last good checkpoint, replay from `wal_seq + 1`, loud on
  every step. A FAULTED stream skips an unrecoverable ORDER gap (orders are
  drop-safe: the gateway answers TIMEOUT for an order nobody answered and the
  client places it again under a new cid; a same-cid resend never reaches the
  ME) and counts it in `/metrics`; fills are never skipped.
- User-keyed state the ME has to hold (reduce-only needs the user's net
  position) goes in one separable module, never spread through the loop: it
  does not belong in a per-symbol process and is slated to move out.
- Tests drive the ME over real sockets and a real WAL (`tests/`); a
  `known_defect_*` test pins an open `BUGS.md` entry. Benches run with the
  cluster stopped. The demo shows networking and recovery, never the book's
  own benchmark again.

## Docs (which file answers which question)

| File | Question |
|---|---|
| `README.md` | what / how to run: one-line pitch, the accept-path number with its caveat, env table, invariant → panic-message mapping, gotchas |
| `ARCHITECTURE.md` | how it is built: trust boundary, measured table, module layout, the loop's steps, cancel index, WAL-crash policy, fan-out, dedup, config reload, checkpoint + replay, FAULTED skip |
| `notes/*.md` | why each choice, Problem → Fix → Cost, indexed by `notes/README.md` with its through-line ("trust the boundary above, own the record below") |

No `compare/` here. Dated numbers live in root `reports/` (publication run
`reports/20260716_book-matching-publication.md`, tail run
`reports/20261020_matching-p99.md`); the Criterion harness is `benches/`
(shared `benches/harness.rs`, one `Me` fixture).

## Keeper sections — do not regress

- **Fixed-work evidence stays empirical.** `match_depth_bench.rs` varies the
  preloaded depth while one non-draining fill stays constant. Describe the
  observed range; never turn it into a general O(1)-in-depth claim.
- **Every published latency table has p50 and p99 from the same run.**
- **FIFO / time priority is the fairness invariant.** The queue discipline is
  rsx-book's; this crate preserves it through event buffer → WAL → replay so
  match order == on-disk order == replay order
  (`notes/fifo-time-priority.md`; `specs/2/6-consistency.md` #1, #3, #5).
- **The trust boundary: the ME repeats no policy validation.** Gateway and
  risk own auth, margin and position limits; `rsx-book` still enforces
  structural sign/tick/lot invariants (`notes/trust-boundary.md`).
- **Preallocated receive/book/event path.** Zero-copy `try_recv_with`, a
  fixed event buffer drained in place, slab order slots, cached clocks
  (`notes/zero-heap.md`). Never per-order `Vec`/`Box` allocations.
- **Authoritative-WAL crash policy.** Fill-path WAL appends `.expect(...)`
  with named-invariant messages; cast sends warn and continue
  (`notes/authoritative-wal.md`). A silently lost fill is unrecoverable.
- **SEQ-1 one-CRC fan-out.** Each record framed once (one WAL seq, one CRC),
  fanned to WAL and both cast streams; every record consuming a seq goes to
  both streams so no stream has a hole (`notes/one-crc-fanout.md`).
- **Each `notes/` file keeps Problem → Fix → Cost; `notes/README.md` keeps
  its through-line.**

New design decision → a `notes/` file + a row in `notes/README.md`. New
measured number → `reports/YYYYMMDD_*.md`, then quoted with bench name and
caveat. Matching-algorithm changes belong in `rsx-book` and its `notes/`.
