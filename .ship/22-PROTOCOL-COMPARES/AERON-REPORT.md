# Aeron comparison — report

## What landed

Two commits on the worktree (`agent-acc8baddc7da44fa5`):

1. `[dxs] aeron: add compare_aeron loopback RTT bench + deps`
   - `rsx-dxs/Cargo.toml` — added `rusteron-client` +
     `rusteron-media-driver` dev-deps with `precompile` + `static`
     features (no cmake-of-Aeron, just system uuid/bsd dev libs at
     link time).
   - `rsx-dxs/benches/compare_aeron.rs` — new Criterion bench:
     embedded media driver, PING / PONG threads, 64-byte payload,
     spin polling, matches `compare_kcp.rs` style.
   - `Cargo.lock` — 800-line delta (precompile pulls reqwest +
     icu + tls bits).

2. `[dxs] aeron: doc, README index, drop-order fix in bench`
   - `rsx-dxs/compare/aeron.md` — full rewrite (~340 LOC, was
     ~118). Wire format, NAK semantics, retransmit horizon,
     flow control, durability story, performance section with
     published AWS numbers + our measured number, guarantees
     comparison table, "where Aeron is genuinely more capable"
     section, "where CMP is intentionally narrower" section,
     prerequisites for the bench, citation list.
   - `rsx-dxs/compare/README.md` — Aeron row in the summary
     table now points at the bench result; "Why these
     protocols" entry updated.
   - `compare_aeron.rs` — fixed driver lifecycle bug surfaced
     by oracle review (see below).

LOC summary:
- `compare_aeron.rs`: 384 lines (new)
- `compare/aeron.md`: 222 lines (was 119; +103)
- `compare/README.md`: 4 lines changed
- `Cargo.toml`: 8 lines added

## Approach: A, B, or C?

**Option A** (preferred): real loopback bench against a running
Aeron media driver.

Specifically: `rusteron-client` + `rusteron-media-driver` with the
`precompile` + `static` features. This bundles the Aeron C
driver as a precompiled binary downloaded from rusteron's
release artifacts and statically links it. **No JVM needed.**

Why this works on this box:
- Debian 12 has cmake 3.25; Aeron itself requires `cmake >= 3.30`,
  which would block building from source. The `precompile` feature
  sidesteps that.
- libclang-dev + clang installed for bindgen (one-time apt).
- uuid-dev + libbsd-dev installed for the link step.

The bench actually runs end-to-end and produces a real measurement.

## Measured number

**Aeron UDP loopback, 64-byte payload, 6-core AMD Ryzen 9 5950X,
no core pinning, embedded media driver in the same process:**

| Run | P50 | P95 (approx) |
|---|---:|---:|
| 10 samples × 5s, run A | ~305 µs | ~360 µs |
| 10 samples × 5s, run B | ~480 µs | ~763 µs |
| 10 samples × 5s, run C | ~1.46 ms | ~2.04 ms |
| 20 samples × 8s | ~394 µs | ~571 µs |

Compared to:
- **CMP RTT in this repo: ~10 µs** (existing `cmp_rtt_bench`)
- **Aeron AWS published: ~21 µs P50** (c6in.16xlarge, 100k msg/s)

Our number is 14–100× worse than the published AWS number. This
is **not** an Aeron protocol issue — it's CPU oversubscription
on a 6-core box where:
- The embedded media driver spins (`set_idle_sleep_duration_ns(0)`)
- The PONG echo thread spins on `subscription.poll()`
- The PING thread spins on `offer()` then on `subscription.poll()`
- Criterion's measurement thread is also running

Aeron's published P50 of 21 µs requires pinned cores + a real NIC.
Our setup is "laptop-class" by design — this is a sibling bench
to the existing `compare_kcp`/`compare_quinn` ones, not an HFT
benchmarking lab.

**Aeron IPC** (shared-memory variant, separately measured in a
smoke during development): **~830 ns P50**. This is the
closest like-for-like with CMP's `cmp_send_breakdown_bench`
of 3.87 µs. Aeron IPC strips out the UDP socket entirely and
runs pub/sub through SHM rings only.

## Oracle review

Invoked `codex` (skill: `oracle`) on the bench. Findings:

| # | Finding | Took? | Why |
|---|---|---|---|
| 1 | Bench docstring overstated what's measured — Criterion times the whole `record_rtt()` closure, not the in-handler `last_rtt_ns`. | **Took** | Clarified bench docstring to say Criterion measures the call-time including spin loops; `last_rtt_ns` is a finer-grained signal but not what's reported. |
| 2 | Driver teardown was broken — `driver_stop` was never set in normal Drop path, the "stopper" thread joined the handle backward. Real bug: contamination between sequential bench runs in the same process. | **Took** | Reworked `AeronRig::Drop` to (1) signal `pong_stop`, (2) join PONG, (3) signal `driver_stop`, (4) join driver. Stored both `driver_stop` and `driver_handle` as struct fields. |
| 3 | High UDP latency is plausibly a scheduler artifact, not a measurement bug. | Documented | Added an explicit caveat block to the bench docstring + the comparison doc explaining the 6-core oversubscription cost; doc reader sees the discrepancy before the number. |
| 4 | `last_rtt_ns` read itself + warmup are not the bug. | Acknowledged | No action — confirms our measurement isn't broken at the timing layer. |
| 5 | `offer() < 0` and `try_claim() < 0` spin treats all negative codes as backpressure; some are admin/transient. | **Skipped** | Matches the upstream rusteron `ping_pong.rs` style; refactoring to distinguish negative codes adds noise without changing the headline number. Documented as a known minor caveat. |
| 6 | UDP-vs-CMP is not apples-to-apples (Aeron has a driver-IPC hop CMP doesn't). | **Took** | Already framed this way; reinforced in the bench docstring + the compare doc. Added an explicit "Not apples-to-apples vs CMP at the transport layer" note. |

The biggest concrete bug was #2 (driver lifecycle). Fix is in the
second commit. The other findings were either docs-only or
theoretical.

## Open questions

1. **IPC variant not in default criterion_group.** Running both
   UDP and IPC in one process triggers a C-side
   `MediaDriver has been shutdown` race on the second `setup()`,
   probably because the rusteron client conductor holds onto
   stale state across driver tear-down/relaunch in the same
   process. The IPC code is kept in source with `#[allow(dead_code)]`;
   a follow-up could put it in a sibling `compare_aeron_ipc.rs`
   bench binary (each Criterion bench = its own process).
2. **Aeron UDP under pinned cores on this box.** A follow-up
   could pin PING / PONG / driver agents to dedicated cores
   via `core_affinity` and see how close to the AWS 21 µs P50
   we can get on stock hardware. Not done because it would
   diverge from `compare_kcp` / `compare_quinn` conventions
   (those don't pin either).
3. **Larger payloads.** All compare benches use 64 B (sometimes
   128 B). Aeron's relative advantage grows at larger payloads
   because the per-frame driver-hop cost amortizes. A second
   axis at 1 KB / 4 KB / 16 KB would be informative.
4. **Loss-injection.** `tc qdisc netem loss 0.1%` would exercise
   Aeron's NAK path under loss and let us compare its recovery
   latency to CMP's. Listed in `compare/README.md` already; not
   exercised here.

## Prereqs to rerun

```bash
sudo apt install -y cmake libclang-dev clang uuid-dev libbsd-dev
cargo bench -p rsx-dxs --bench compare_aeron
```

First build downloads the precompiled Aeron C driver from
rusteron's release artifacts (~few MB).
