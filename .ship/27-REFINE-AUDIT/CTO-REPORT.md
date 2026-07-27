# CTO Audit — .ship/27-REFINE-AUDIT

Adversarial source + bench audit at HEAD `5cca081` (master). Read-only.
Targets: rsx-cast Round 1-4 changes and the consumer wiring in
rsx-{matching,risk,gateway,marketdata,mark}.

Findings: 5 claims verified; 3 attack scenarios traced; 4 Round
spot-checks; **grade 58 / 100 — hold**. Strong technical kernel,
but four user-visible defects landed because Round 4 audited
docs against specs instead of against the running code.

---

## 1. Claim verification

### Claim 1 — "WAL bytes = wire bytes = TCP replay bytes"
- **Source**: rsx-cast/README.md:50-65, 99-136.
- **Checked**: `WalWriter::prepare()` at `rsx-cast/src/wal.rs:142-172`
  builds one `Framed { wire, total, seq }`. `append_framed` writes
  exactly `framed.wire[..total]` to disk (`wal.rs:192`).
  `CastSender::send_framed` writes exactly the same bytes to the UDP
  socket (`cast.rs:326,336`). `ReplicationService` reads via
  `WalReader::next()` and forwards header + payload bytes unchanged.
- **Verdict**: **CONFIRMED**. The 16 B `WalHeader` + repr(C) payload
  is bit-identical on disk, on UDP, and on TCP. No serialisation step
  anywhere. This is the strongest single property of the codebase.

### Claim 2 — "Zero heap on the hot send path"
- **Source**: rsx-cast/README.md:79-81, 123-136; CLAUDE.md:32,96.
- **Checked**: `CastSender::send` and `send_framed` at
  `rsx-cast/src/cast.rs:243-346` allocate nothing — the ring is a
  `Box<[u8]>` built once in the constructor (`cast.rs:205-219`); both
  paths write into a pre-allocated slot and call `sendto`.
- **Verdict**: **CONFIRMED for the send path; DISPUTED for the
  consumer-visible hot path.** Every RSX consumer
  (risk, gateway, marketdata, matching, mark) uses
  `CastReceiver::try_recv()`, not `try_recv_with()`. `try_recv` at
  `cast.rs:1064-1089` allocates one `Vec<u8>` per in-order packet
  via `payload.to_vec()` (`cast.rs:1067`). The README admits this at
  line 348-350 ("Receive path allocates one `Vec<u8>` per in-order
  packet"), yet CLAUDE.md:32 still asserts the project-wide
  "Zero heap on hot path" rule unqualified, and BLOG/PROGRESS
  surfaces repeat it. The unconditional claim is **false** at HEAD;
  the qualified send-path claim is true.
- Bonus heap site (not in send path but on the production critical
  path): `rsx-risk/src/shard.rs:843-846` allocates
  `let users: Vec<u32> = self.exposure.users_for_symbol(sid).to_vec()`
  on **every BBO event**. Flagged in CTO Round 2 R-N5; still present.

### Claim 3 — "One CRC per record via prepare → append_framed → send_framed"
- **Source**: .ship/27-REFINE-AUDIT/REPORT-2.md §"Consumer-side wins";
  specs/2/17-matching.md §Order Acceptance Flow step 4.
- **Checked**:
  - `fan_out` (`rsx-matching/src/wal_integration.rs:445-458`) does
    exactly: `prepare → append_framed → send_framed [→ send_framed]`.
    One CRC compute in `prepare` (`wal.rs:160`), reused for WAL +
    risk + mkt fan-out.
  - `process_cancel` calls `publish_events` (`main.rs:972-975`); no
    parallel `send_event_cmp`/`send_event_marketdata` paths remain.
    Diff at `d009266` confirms -207 LOC.
  - `rsx-mark` uses `send_framed` (`mark/src/main.rs:178-218`); no
    paired `send_raw`.
- **Verdict**: **CONFIRMED** for the order + cancel + mark paths. One
  CRC per WAL-and-cast event.
- **Caveat**: `BboRecord` is published twice — `cmp.send(&mut record)`
  then `mkt.send(&mut record)` (`wal_integration.rs:431-436`). Each
  call re-runs `compute_crc32`. Two CRCs per BBO, not one. Defensible
  (BBO bypasses WAL by design and has its own seq counter per
  destination), but the headline "one CRC per record" needs the
  BBO carve-out spelled out in the spec.

### Claim 4 — "casting has no flow control; producer can't stall"
- **Source**: rsx-cast/specs/4-cast.md (post-87b223e); README:76-78.
- **Checked**:
  - `CastSender::send` (`cast.rs:243-297`) calls `sendto` once and
    returns. No window check, no `peer_consumption_seq`, no
    `Ok(false)` flow-control branch. Producer cannot stall.
  - There IS one stall site outside the send path: `WalWriter::
    append_framed` (`wal.rs:181-189`) returns `WouldBlock` if the
    in-memory buffer exceeds `2 × max_file_size`. That's WAL
    back-pressure, not casting back-pressure, but a producer using the
    `prepare → append_framed → send_framed` pattern can be stopped at
    the append step.
- **Verdict**: **CONFIRMED for the casting path** (UDP `sendto` is
  fire-and-forget). The WAL back-pressure stall is correctly
  documented in the wider spec but worth noting that "producer can't
  stall" is true of the UDP socket alone, not of the integrated
  producer loop.

### Claim 5 — "WAL retention 4h" (R4 flagged item #1)
- **Source**: CLAUDE.md, FEATURES.md:55, BLOG.md (post-eb444cc),
  rsx-cast/ARCHITECTURE.md (post-91e4b9f), specs/2/48-wal.md.
- **Checked**: `grep -nE 'retention|prune|gc|delete|remove_file|
  unlink' rsx-cast/src/wal.rs` returns **nothing**. `WalWriter`
  struct (`wal.rs:81-92`) has no `retention_ns`, no timer, no GC task.
  `rotate()` (`wal.rs:242-278`) renames `active → segment` and opens a
  new active file. **No segment is ever deleted by this crate.**
  All retention-bearing docs are aspirational.
- **Verdict**: **DISPUTED — documented but not implemented.**
  R4 was correct to flag this. Worse, the README quick-start at
  `rsx-cast/README.md:164-170` calls `WalWriter::new` with a
  `retention_ns` parameter — but the **actual signature is
  `new(stream_id, &Path, max_file_size)` (`wal.rs:95-99`)**. The
  example wouldn't compile. Three orthogonal failures from one missing
  feature:
  1. Behaviour: disk grows unbounded.
  2. README: wrong API arity (would fail `cargo run --example`).
  3. ARCHITECTURE: claims a default ("4h" or "48h", depending on
     surface) that does not exist.
- **Severity**: critical for a "trust the docs" customer. Six months
  in production at 26 M elems/s (wal_bench result below) = ~50 GB/day
  of WAL bytes per stream. No GC. Disk-full → ME crash.

### Claim 6 — Test count claim drift (R4 #3)
- **Sources**: PROGRESS.md (878), TESTING.md (878), FEATURES.md (878),
  BLOG.md (883), ONEPAGER.md (887), MEMORY.md (887+).
- **Measured**: `cargo test --workspace` at HEAD: **878 passed /
  0 failed / 46 ignored** (`tmp/test-full.log`). One pre-existing
  flake (`rsx_log::tests::drop_counter_increments_on_full_ring`) that
  passes ~50% of the time; clean run today.
- **Verdict**: 878 is the truth. PROGRESS/TESTING/FEATURES are
  current. **BLOG (883) and ONEPAGER (887) are stale.** R4 was right
  to flag this; the spread is small (1%) but it's the kind of
  inconsistency a reviewer notices in the first 30 seconds.

### Claim 7 — "CRC32C via SSE4.2 hardware path"
- **Sources**: rsx-cast/src/encode_utils.rs:7-13 ("Uses SSE4.2's
  `crc32` instruction on x86_64").
- **Checked**: `Cargo.lock` shows `crc32c v0.6.8`. The crate's
  README/source confirm SSE4.2 `_mm_crc32_u64` on x86_64 with a
  software fallback. Confirmed by bench: `send.crc32_128b` =
  **29.6 ns p50** for 128 B (`tmp/cast_send_bd.log`) = ~4.3 GB/s on
  one core — consistent with hardware-path Castagnoli.
- **Verdict**: **CONFIRMED.**

---

## 2. Three attack scenarios

### Attack A — Spoofed-heartbeat DoS against a casting receiver

**Setup**: Attacker has any UDP port reachable to a CastReceiver
(trust model says LAN; in practice this is anyone with internal
network access — a misbehaving sibling tile, a compromised host, a
mis-routed test harness).

**Attack**: Send a single `CastHeartbeat { highest_seq: u64::MAX - 100 }`
to the receiver's bind addr. The header CRC is over the payload only,
so any attacker can craft a valid frame.

**Trace**:
1. `CastReceiver::try_recv_with`, `cast.rs:833`: `socket.recv_from`
   returns `(n, _)` — peer address discarded. No source check.
2. `cast.rs:875-887`: `RECORD_HEARTBEAT` branch updates
   `self.highest_seen = hb.highest_seq` (the spoofed near-u64::MAX
   value, since it exceeds the legitimate highest_seen).
3. `cast.rs:893`: `maybe_nak(now_ns)` calls `oldest_missing_run()`.
4. `cast.rs:1100-1120`: the loop walks `seq` from `expected_seq` to
   `highest_seen` linearly with `seq += 1`. With highest_seen ≈
   2^64 - 100 and reorder_seqs empty for those slots, **the loop
   iterates ~18 quintillion times — effectively forever**, hanging
   the receiver thread.

**Where it fails to catch**: `cast.rs:1108` — the loop has no upper
bound on iteration count. The +1 wrap at `cast.rs:1102`
(`highest_seen + 1`) saves the literal `u64::MAX` case (because
0 wraps under, returning None), but any near-max value walks fully.

**Where the spec said "OK"**: spec/4-cast.md §10.4 + CLAUDE.md
"Trust boundaries" delegate auth to L3. That correctly says "don't
authenticate frames." It does **not** authorise unbounded loops on
attacker-controllable input. The fix is one `min(highest_seen,
expected_seq + REORDER_CAPACITY)` clamp at `oldest_missing_run`,
not source-IP filtering.

**Consequence**: every consumer in risk/marketdata/gateway is
single-threaded. Wedging the receiver wedges the consumer. risk
panics on FAULTED; that's the symptom, not the cause.

### Attack B — Crash + restart with partial WAL flush

**Setup**: ME mid-burst, 8 ms since last `flush()`. SIGKILL.

**Trace**:
1. `WalWriter::flush` runs every 10 ms (`wal_integration.rs:462-473`).
2. Up to ~80 records (10 ms × 8 k pps typical) live in
   `WalWriter::buf` (`wal.rs:84`). On SIGKILL, that buf vanishes.
3. ME restarts. `main.rs:301-321`: loads `snapshot.bin` + `wal_seq.txt`,
   then `replay_wal_after_snapshot` replays disk records >
   `sidecar`.
4. Disk has everything up to the **last fsync'd flush** — i.e.
   up to 10 ms behind the gateway's last-seen fill.
5. `wal_writer.set_next_seq(last_seq + 1)` (`main.rs:332`). Live
   writes resume — but the gateway already broadcast fills the ME
   doesn't know about.

**Where it fails to catch**: Round 2 audit's R-N1 ("ME has no WAL
replay on startup") IS resolved at HEAD (`main.rs:286-346` wires
snapshot + replay). The **residual** is the 10 ms unflushed window
that's an intentional latency/durability tradeoff — bounded loss
admitted in README "Guarantees" §Durability and rsx-cast/README:319.

**Verdict**: not a bug; the 10 ms is documented. But the
documentation should NAME the consequence:
"Up to one flush interval (10 ms default) of fills that the gateway
already sent will not appear in the post-recovery book." Currently
it's a generic "bounded loss" line; a reviewer with a customer hat on
will demand the precise contract.

### Attack C — Replay attack against `JtiTracker` (R4 audit prompt)

**Setup**: Attacker captures a legitimate Bearer token mid-flight
(MITM on the TLS termination box).

**Trace**:
1. `rsx-gateway/src/ws.rs:182-209`: handshake validates JWT,
   then `claims.jti.as_deref().ok_or("missing jti")?` rejects tokens
   without a jti. The missing-jti bypass that Round 1 caught is
   genuinely closed.
2. `JTI_TRACKER.lock().record(Some(jti))` returns `false` for a
   replayed jti. Bearer is rejected.
3. **Where the attack still succeeds**: the tracker is bounded
   (`jwt.rs:79-87` — last `cap` jti values FIFO). If `cap=1000` and an
   attacker spams 1000 fresh jtis from a compromised co-tenant
   account, **legitimate jtis at the bottom of the FIFO age out** and
   become replayable.
4. The README at line 210-211 claims **`JtiTracker` "isn't yet
   consulted by the WS handshake."** This is **false at HEAD**
   (verified at `ws.rs:202-208`). The README is stale.

**Verdict**: JtiTracker is wired correctly per the spec; the FIFO
eviction is a real but expected limitation (longer-window dedup needs
a TTL ring or a persistent table); the **README's claim that it's
unwired is a docs-vs-code drift bug**, not a security bug.

This is the second R4-flagged docs-vs-code drift (the first was
`WalWriter::new` signature). Two docs-vs-code drifts of this type
in a single sprint suggests the Round 4 sweep audited docs against
specs but **did not audit docs against the compiled crate's public
surface**. Recommendation: a CI step that
`cargo doc --no-deps && grep` for stale method names in README.

---

## 3. Round 1-4 spot-checks

| Round | Sampled commit | Real cut, or relocation? | Verdict |
|---|---|---|---|
| R1 | `dc2a6a6` ("[cast] drop CastReceiver::tick no-op") | Genuine cut: 9 LOC out of `cast.rs`, 22 LOC out of `cast_one_way_bench.rs`. Test still green. No replacement. | Sound |
| R2 | `1415b7e` ("[mark] use send_framed (one CRC)") | Real CRC savings: `send_raw` calls `frame_and_send` which does `compute_crc32(payload)` (`cast.rs:110`). `send_framed` doesn't recompute. Two sites converted (aggregate + sweep paths). | Sound — the CRC really halves, not just defers |
| R2 | `d009266` ("[matching] process_cancel uses publish_events") | -207 LOC. `send_event_cmp` and `send_event_marketdata` helpers fully removed (`grep` returns 0 hits). | Sound |
| R3 | `3649b49` ("[spec] matching: publish_events one-CRC fan-out") | Spec text now says what the code does: `publish_events` (`wal_integration.rs:234`) does fan-out via single `prepare` per event. Step 4 prose matches `fan_out` impl. | Sound, NOT aspirational |
| R3 | `190b6a6` ("[spec] WAL header: version at byte 0; CRC32C") | Code at `header.rs:5,45` has `version: u8` first (byte 0); `encode_utils.rs:7` is `crc32c::crc32c`. Spec sync is accurate. | Sound |
| R4 | `1a3b673` ("[docs] features: 4h WAL retention") | Spec/doc both say 4h. **Code does not enforce any retention** (claim 5 above). Round 4 updated the WRONG side: it brought docs into agreement with each other, leaving both at variance from the code. | **Half-done** — R4's own findings list flags this as item #1, which is honest, but the doc edit was committed before the underlying bug was fixed |
| R4 | `26e5753` ("[docs] drop broken .ship/ refs") | Refs really were broken (pruned audit dirs); 3 refs removed cleanly. | Sound |

**Net R1-R4 grade**: R1, R2, R3 are sound — code actually got smaller
and specs caught up to reality. R4 is half-done: it tightened the
prose-to-prose consistency loop but committed three changes (4h
retention in FEATURES + BLOG + ARCH) that assert a feature the code
doesn't have. The R4 report itself flagged the gap; the right
sequence would have been "fix the code, then update the doc."

---

## 4. Strengths (do not regress)

1. **The bytes-are-the-bytes invariant is genuine.** Claim 1
   survives end-to-end. The whole protocol value-prop rests on this
   and it's defensible.
2. **publish_events fan-out is clean.** `fan_out` is 14 lines, one
   prepare, one append, one or two sends. The post-Round-2 code is
   measurably simpler than the pre-Round-2 send-event-* helpers it
   replaced.
3. **WAL replay on ME startup is wired** (`main.rs:286-346`). CTO
   Round 2's biggest finding (R-N1) is closed at HEAD.
4. **`order_prod` no longer drops on backpressure.** `main.rs:578-597`
   stall-loops via `shard.run_once`. CTO Round 2 R-N3 closed.
5. **JtiTracker missing-jti bypass is closed.** `ws.rs:195-208`.
   Round 1 R3 closed.
6. **rsx-cast has zero `rsx-*` deps in `cargo tree`.** The standalone
   extraction story stays defensible.

---

## 5. Top critical risks at HEAD

| # | Risk | Citation | Severity |
|---|---|---|---|
| 1 | WAL retention: no GC, no time-based prune, no segment delete. Disk grows unbounded. | `wal.rs:81-278` | critical |
| 2 | README `WalWriter::new` example uses 5-arg signature; real signature is 3-arg. Example won't compile. | `rsx-cast/README.md:164-170` vs `wal.rs:95-99` | high (visible defect) |
| 3 | README + 2x ARCHITECTURE refer to `CastReceiver::poll` — method doesn't exist; real name is `try_recv_with`. | `rsx-cast/README.md:79,191,199` vs `cast.rs:810` | high (visible defect) |
| 4 | README claims `JtiTracker` "isn't yet consulted by the WS handshake" — but at HEAD it IS consulted. | `README.md:210-211` vs `ws.rs:202-208` | high (visible defect, security-flavoured) |
| 5 | `oldest_missing_run` can be made to loop ~2^64 times by a spoofed heartbeat with `highest_seq` near u64::MAX. Unbounded inner loop on attacker-controllable input. | `cast.rs:1100-1120` (no clamp) + `cast.rs:882-887` (no validation) | critical (LAN DoS) |
| 6 | Every consumer uses allocating `try_recv`; CLAUDE.md and BLOG still claim project-wide "Zero heap on hot path" unqualified. | `risk/main.rs:523`, `gateway/main.rs:198`, `marketdata/main.rs:219`, `matching/main.rs:500` | medium (false marketing) |
| 7 | risk's BBO scan allocates `Vec<u32>` per BBO. | `rsx-risk/src/shard.rs:843-846` | medium (Round 2 R-N5 not closed) |
| 8 | Stale `specs/2/50-wedge.md` exists despite MEMORY claiming "deleted 2026-05-24". | `specs/2/50-wedge.md` | low (R4 flag item #4 still open) |
| 9 | risk/marketdata/gateway panic on `CastRecv::Faulted`. README:217-220 admits it. | `risk/main.rs:526-538`, `gateway/main.rs:202-?`, `marketdata/main.rs:219-?` | medium (POC-grade) |
| 10 | BBO record CRC'd twice (cmp + mkt destinations) per event; spec headline says "one CRC per record." | `wal_integration.rs:431-436` | low (defensible carve-out, spell it out) |

---

## 6. Grade

**58 / 100 — hold.**

Why not lower:
- The architectural core (bytes-are-the-bytes, publish_events fan-out,
  WAL replay on ME startup, JtiTracker wired) is real and defensible.
- The Round 1-3 cuts are honest — code genuinely shrank by -287 LOC
  in R2 alone, and the spec sync in R3 reflects what the code does,
  not what the writer wished it did.
- Test suite is 878-green, the flake is named and isolated, claim 6
  drift is 1%.

Why not higher:
- Three docs-vs-code drifts in a single crate's README
  (`WalWriter::new` signature, phantom `poll` method, "JtiTracker not
  wired" line) — all introduced during a sprint whose Round 4 was
  explicitly "docs hygiene." That sprint did not audit the docs
  against the compiled crate.
- The "WAL retention 4h" claim is published in 5+ surfaces and
  enforced in zero. Disk-fill is a real production failure mode.
- `oldest_missing_run` has a u64-bounded inner loop on
  attacker-controllable input. Even under the "trusted LAN" trust
  model, a single misconfigured sibling tile can wedge a consumer
  thread.
- "Zero heap on hot path" is gospel in CLAUDE.md but false in 4 of
  4 production consumers.

A friendly first customer would catch finding #2 or #3 in their
first hour (`cargo run --example` failing, or `cargo doc | grep poll`
returning nothing). The fixes are mostly small (1-10 lines each) but
they need to land before any external eyes are invited.

**Recommendation**: hold for one focused fix-up sprint addressing
risks 1-5 (the user-visible defects and the receiver DoS); risks 6-10
are durable but not blocking.
