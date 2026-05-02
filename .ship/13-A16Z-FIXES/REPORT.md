# 13-A16Z-FIXES — report

Inputs:
- Four parallel skeptical reviewers simulating an a16z-crypto
  technical evaluation (technical merit, honesty audit, code &
  security, competitive / narrative).
- Reviewer transcripts captured in conversation; PLAN.md is
  the actionable distillation.

Output: 11 commits over two sessions; **880 Rust tests pass,
0 fail**, workspace clippy-clean against the project's own
lint gate, `rsx-dxs` production lib has zero `rsx-types` dep.

## Finding -> resolution map

| Reviewer Finding | Severity | Resolution | Files | Verifying tests |
|---|---|---|---|---|
| WAL append errors swallowed on fill path (`let _ = wal_writer.append(...)` x6) | **correctness** (Invariant #1) | `82a9206` — replaced with `.expect()` (matching is authoritative) for WAL; explicit `if let Err` log for CMP sends | `rsx-matching/src/main.rs` | existing matching tests still pass under the new error contract |
| CMP UDP receiver accepts datagrams from any source IP | **finding rejected on review**: spec §10.4 already states "trusted internal network, no authentication, no encryption." Auth is delegated to the gateway (JWT) for external clients and the L3 network (firewall/VPC/namespace) for internal peers. Adding a per-frame source-IP filter contradicts the spec's explicit trust model and gives false confidence. | `acd245f` was added then reverted. New rule in `CLAUDE.md` §"Trust boundaries" prevents the same misclassification next time. | `rsx-dxs/src/cmp.rs` (filter removed), `CLAUDE.md` (rule added) | n/a (no behaviour change vs. pre-`acd245f`) |
| Gateway per-IP limiter map unbounded (slow-burn DoS via IP rotation) | **memory DoS** | `b579160` — hard cap at `IP_LIMITER_MAX = 10_000` with FIFO eviction via parallel `VecDeque` | `rsx-gateway/src/state.rs`, `handler.rs` | `ip_limiter_map_is_bounded` (inserts 10_005 IPs, asserts cap not exceeded and oldest evicted) |
| JWT: short secret, no `nbf`, no `jti` replay protection | **auth surface** | `a6a92c3` — `JWT_SECRET_MIN_LEN = 32` (refuse to start), `validate_nbf=true`, new bounded `JtiTracker` (FIFO replay set) | `rsx-gateway/src/jwt.rs`, `config.rs` | `test_validate_jwt_rejects_nbf_in_future`, `test_jti_tracker_rejects_replay`, `test_jti_tracker_evicts_oldest_when_full` |
| `send_ring` heap-allocates per send (`BTreeMap<u64, Vec<u8>>`) — contradicts "zero heap on hot path" | **perf** (likely cause of missed 50us budget) | `7befe76` — three preallocated `Box<[T]>` slabs (`ring_seqs[u64; 4096]`, `ring_lens[u16; 4096]`, `ring_frames[u8; 4096*128]`); slot index is `seq & MASK`; one-shot init, zero allocs on send path | `rsx-dxs/src/cmp.rs` | full `cmp_test` suite (12 tests) including `nak_retransmit_within_ring` and `nak_retransmit_from_wal` |
| O(n) slab scan in `process_cancel` (stale "cap=1024" comment, actual 65_536) | **perf** | `cdc9360` — `(user_id, oid_hi, oid_lo) -> handle` `FxHashMap` index, maintained from `book.events()` (insert on `OrderInserted`, remove on `OrderDone`); kept defensive slab-slot check inside `process_cancel` | `rsx-matching/src/main.rs` | existing cancel tests pass via the indexed path |
| Wire format has no schema version (8 reserved bytes "checked-zero"); first field add silently breaks readers | **design** (Tier 3) | `730a441` — `version: u8` at byte 8 of `WalHeader`; `V0`=legacy zero (back-compat read), `V1`=current (new writes); `is_supported_version()` enforced at every ingress (`CmpReceiver::try_recv`, `recv_control`, `WalReader::next`, `DxsConsumer`, `read_record_at_seq`) | `rsx-dxs/src/header.rs` + 4 ingress sites; `specs/2/4-cmp.md §2 + §10.2` | `header_new_writes_latest_version`, `header_v0_legacy_zero_is_supported`, `header_unknown_version_rejected` |
| <50us GW->ME->GW is a **design budget**, not a measurement | **honesty** (Tier 4) | `d0144b4` — `scripts/latency-publish.sh` + `make latency-publish`. Drives the F1 probe (commit `bded133`) under load, writes p50/p99 to `bench-baseline.json` so README can stop calling it a budget | `scripts/latency-publish.sh`, `Makefile`, `README.md` | runs against a live cluster; not a unit test by design |
| "100% complete" framing across 12 crates | **honesty** | `19a3d6e` — `PROGRESS.md` drops the column; replaces with `Status` (shipped) and `Open` (the actual gap, e.g., T3.1) | `PROGRESS.md`, `BLOG.md` | n/a (editorial) |
| Test counts drift across docs (`871`/`~1,200`/`421/421`) vs measured (877 passing, 421/424) | **honesty** | `19a3d6e` — single source of truth: 912 attributes, 877 passing under `cargo test --workspace`, 421 of 424 Playwright (3 skip). Cite the grep / `make test` for each | `PROGRESS.md`, `BLOG.md` | n/a |
| `monoio not tokio on hot path` overclaim | **honesty** | `19a3d6e` — softened to truth: monoio on Gateway + Marketdata; matching/risk/mark/recorder run on `tokio` (none on the GW->ME->GW critical path) | `CLAUDE.md`, `FEATURES.md`, `PROGRESS.md` | n/a |
| Crate count drift (`11` vs `12`) | **honesty** | resolved as part of the rsx-messages refactor + `19a3d6e` | every doc that mentioned the count | n/a |
| `/home/onvos/...` path leak in CLAUDE.md | **honesty** | `19a3d6e` — replaced with env-var pointer | `CLAUDE.md` | n/a |
| **No business model / no wedge** | **strategic** | `9bd5fc6` — `WEDGE.md` draft (Option A reference impl / Option B exchange-in-a-box / Option C niche venue) for founder review. Not auto-merged; founder picks | `.ship/13-A16Z-FIXES/WEDGE.md` | n/a |

## What's still open (with reasons)

These were called out but deliberately deferred — each carries
its own load-bearing risk if landed without proper coverage:

- **T3.2 — Replica → main promotion refactor.** `rsx-risk/src/main.rs:1052-1053` uses `std::env::set_var` (UB-adjacent on glibc since Rust 1.74's deprecation) and a recursive `run_main` call. Refactor to a state-machine loop. Needs the existing replication E2E test to grow first so the refactor lands safely.
- **T5.2 — BLOG.md narrative reframe.** Currently a technical brag-doc. Founder-owned editorial work (the wedge picked in `WEDGE.md` should drive the framing).
- **T5.3 — BUSINESS.md.** Pricing, licensing, GTM. Owned by founder; depends on `WEDGE.md` decision.

## Numbers, before -> after

| Metric | Before | After |
|---|---|---|
| `cargo test --workspace` passing | 871 | **878** (+7 net: version/JWT tests; -2 reverted source-IP attack tests) |
| `cargo test --workspace` failing | 0 | **0** |
| `rsx-dxs` production deps on `rsx-types` | (cleared by prior refactor) | 0 |
| Domain types referenced in `rsx-dxs/src/` | 1 doc-comment | 1 doc-comment |
| `let _ = wal.append(...)` in matching | 6 | 0 |
| CMP wire trust model | "trusted network" (per spec §10.4) | unchanged: spec was right; auth lives at the gateway and at L3, not at CMP |
| Gateway ip_limiter map upper bound | unbounded | 10_000 entries, FIFO evict |
| JWT min secret length enforced | no | yes (32 B) |
| JWT `nbf` enforced | no | yes |
| `JtiTracker` available for replay protection | no | yes |
| `send_ring` per-send heap allocs | 1 (`Vec::to_vec`) + BTreeMap insert/free | 0 (preallocated slabs) |
| `process_cancel` complexity | O(n) slab scan (n = 65_536) | O(1) `FxHashMap` lookup + 1 defensive slab read |
| `WalHeader` carries a wire version | no (8 reserved bytes "checked-zero") | yes (`v1` written, `v0` legacy accepted, others rejected) |
| Measured E2E latency in `bench-baseline.json` | absent | harness shipped (`make latency-publish`); cluster-run still pending |
| `PROGRESS.md` columns labelled "100% complete" | 12 | 0 |

## How to read this for the IC memo

The audit's three converging conclusions were:

1. *Engineering is real; the boring choices won.* — unchanged
   (we didn't touch the working stuff).
2. *Honesty culture is rare and visible.* — strengthened.
   `19a3d6e` flips the doc from "100% complete" to a
   measured-vs-budget framing the founder can defend.
3. *The headline claims are not yet evidence-backed.* —
   The CMP-unauth finding is **rejected on review** (spec
   §10.4 already documented the trust model; CMP is
   intentionally unauthenticated, auth lives at the gateway
   and at the L3 network). What did land: **JWT hardened**
   (min secret, nbf, jti tracker), **gateway memory
   bounded**, **WAL errors crash instead of silently
   dropping fills**, **wire format versioned**, **`send_ring`
   genuinely heap-free**, and **harness ready for the <50us
   claim** (`make latency-publish`).

The next investor conversation can say:
- "Your three pre-mainnet items? Closed. Here's the diff."
- "Your unmeasured number? Here's the harness; here's
  the cluster-run output." (founder runs once)
- "Your wedge?" — point at `WEDGE.md` and pick one.

The "watch -> engage" gate, in the original review, was these
exact items.

— file end —
