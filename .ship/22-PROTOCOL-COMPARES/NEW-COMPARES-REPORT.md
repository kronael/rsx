# Protocol Compares — MoldUDP64 + SoupBinTCP (+ ITCH 5.0 + OUCH 5.0)

Date: 2026-05-24
Branch: this worktree
Budget consumed: ~1.5 h of the 4 h cap

## Scope shipped

All four "deliverables" listed in the brief, including the optional
ITCH 5.0 and OUCH 5.0 docs:

| Artefact | Path | Status |
|---|---|---|
| MoldUDP64 doc | `rsx-dxs/compare/moldudp64.md` | NEW |
| SoupBinTCP doc | `rsx-dxs/compare/soupbintcp.md` | NEW |
| ITCH 5.0 doc | `rsx-dxs/compare/itch5.md` | NEW (optional) |
| OUCH 5.0 doc | `rsx-dxs/compare/ouch.md` | NEW (optional) |
| MoldUDP64 bench | `rsx-dxs/benches/compare_moldudp64.rs` | NEW |
| SoupBinTCP bench | `rsx-dxs/benches/compare_soupbintcp.rs` | NEW |
| Cargo bench entries | `rsx-dxs/Cargo.toml` | EDITED |
| Compare README | `rsx-dxs/compare/README.md` | EDITED |

Nothing else touched. No `src/`, no other crate, no existing bench
or compare doc.

## Bench results (quick local validation)

Linux loopback, std `UdpSocket` / `TcpStream`, 64 B payload,
sample-size=30 / measurement-time=2s / warm-up-time=1s.

| Bench | p50 (µs) | Range (µs) |
|---|---|---|
| `moldudp64_rtt_loopback_64b` | ~10–13 | 9–17 |
| `soupbintcp_rtt_loopback_64b` | ~13–16 | 12–20 |

Both well within the predicted sub-30 µs band; consistent with
existing `compare_quinn.rs::tcp_rtt_loopback_64b` (~tens of µs)
and noticeably above the ~2 µs raw-UDP floor (`udp_rtt_bench`).

## Oracle review — substantive findings + fixes

Two real bugs caught, both fixed before commit:

### 1. SoupBinTCP — `write_all` on non-blocking `TcpStream` is unsound

`std::net::TcpStream::write_all` on a non-blocking socket can return
`WouldBlock` (treated as `Err` by `write_all`) or fail after a
partial write. With `.expect("...write")` on every call site, the
bench would panic under TCP send-buffer backpressure or scheduler
jitter even though the framing is correct.

Fix: added `write_all_spin` helper mirroring `read_exact_spin` —
loops on `write()` while `WouldBlock`, panics on hard errors. All
three `write_all().expect(...)` sites in `compare_soupbintcp.rs`
replaced.

### 2. MoldUDP64 — echoer was incrementing `seq` on heartbeats

Per spec, MoldUDP64's 8-byte `seq` field is "sequence number of
the FIRST message in this packet". Heartbeat packets (`msg_count = 0`)
carry no messages, so they report the next-expected message
sequence and do **not** consume one. The echoer was bumping
`echo_seq` on every heartbeat reply, which would have left a
gap in the data-packet sequence after any heartbeat traffic.

Fix: removed `echo_seq += 1;` from the heartbeat branch of the
echoer in `compare_moldudp64.rs`; added a clarifying comment.

(The current bench doesn't actually send heartbeats from the
pinger — but the echoer is now spec-correct if a future iteration
does, and the doc claim about heartbeat semantics in `moldudp64.md`
now matches the code.)

## Design choices worth flagging

- **Unicast, not multicast.** Both new benches use unicast UDP /
  TCP to stay apples-to-apples with `udp_rtt_bench`, `compare_kcp`,
  and the existing `compare_quinn` TCP variant. Loopback multicast
  on Linux would measure IGMP / `IP_ADD_MEMBERSHIP` plumbing, not
  the protocol's framing cost. The MoldUDP64 doc states this
  explicitly.

- **Full parse + reframe on the echo side**, not raw byte mirror.
  The MoldUDP64 echoer parses the header and per-message length
  prefix, builds a fresh packet with its own seq counter. The
  SoupBinTCP echoer parses the 3-byte header, reads the announced
  payload, then frames an `S`-typed echo. Both directions exercise
  the framing layer in full.

- **End-of-session shutdown.** MoldUDP64 uses `msg_count = 0xFFFF`
  (header only). SoupBinTCP uses packet_type `Z`. Both are the
  protocol's documented "stream done" markers, so we get a clean
  echoer-thread exit without ad-hoc sentinels.

- **Pinning deferred.** Per the brief, a parallel sub is adding
  `core_affinity` across the bench suite. Each new bench has a
  `TODO(pinning):` comment next to its thread spawn. No pinning
  code in this branch — it would only conflict with the merge.

## Honest framing in docs

For each protocol I called out where it is **genuinely stronger**
than CMP (MoldUDP64: native multicast fan-out, multi-msg-per-
packet, NAK suppression; SoupBinTCP: 3-byte header vs 16-byte WAL
header, explicit session) before listing where CMP wins. The
"Stronger than CMP / Weaker than CMP" subheaders in both docs
are deliberate — no straw-manning.

## What did NOT get done

- The parallel core_affinity merge isn't done — these benches will
  pick it up there.
- The `tc netem`-driven loss-recovery variants for MoldUDP64 (would
  need a separate request-server channel implementation, easily
  another half-day of work). Left as future work; not in the brief.
- No iggy-style cross-protocol throughput benches — RTT only, per
  the brief.

## Files touched (relative paths)

NEW:
- `rsx-dxs/compare/moldudp64.md`
- `rsx-dxs/compare/soupbintcp.md`
- `rsx-dxs/compare/itch5.md`
- `rsx-dxs/compare/ouch.md`
- `rsx-dxs/benches/compare_moldudp64.rs`
- `rsx-dxs/benches/compare_soupbintcp.rs`
- `.ship/22-PROTOCOL-COMPARES/NEW-COMPARES-REPORT.md` (this file)

EDITED:
- `rsx-dxs/Cargo.toml` (two new `[[bench]]` stanzas)
- `rsx-dxs/compare/README.md` (4 new table rows, 2 new
  `cargo bench` lines, 4 new "why these protocols" bullets)
