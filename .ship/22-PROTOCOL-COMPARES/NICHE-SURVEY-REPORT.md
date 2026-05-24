# Niche-survey meta-report

Written after the long-tail census in `rsx-dxs/compare/niche.md`.

## Scope and count

Final entries in `niche.md`: ~70 named projects across 8 categories
(NAK reliable UDP, ACK/QUIC, persistent log, trading wire formats,
DPDK/kernel-bypass, multicast/pub-sub, gossip, per-language).

Candidates touched but dropped during triage: roughly twice that —
small-star toy repos, single-commit graduate projects, duplicate
QUIC implementations, derivative forks, FIX engines that are just
sockets, mailing-list-era IETF drafts with no code. Total
result-pages skimmed across WebSearch + WebFetch was on the order
of ~700 (against the user's "~1000" target — the additional 300
would have been Chinese / Japanese / Russian source dives, see
"thinner than expected" below).

Promotion shortlist: 10 candidates ranked into Strong-Yes (4),
Yes-if-time (3), Maybe (3), with explicit drop rationale for the
QUIC fanout and the gaming-RUDP fanout.

## Category richness — versus prior expectations

Richer than expected:

- **QUIC implementations.** Easy to find 10+ production-grade
  ones (msquic, mvfst, lsquic, ngtcp2, picoquic, s2n-quic, neqo,
  quiche, aioquic, tquic, xquic). They're all RFC 9000 so each
  individual one is low-information — the *count* matters.
  Citation strategy: collapse into the existing `quinn.md`
  rather than 11 separate docs.

- **Gaming RUDPs.** Glenn Fiedler's stack
  (netcode.io + reliable.io + yojimbo) is conceptually tight and
  well-cited. Plus LiteNetLib (3.5k stars, active), naia,
  laminar, Tachyon, ENet, RakNet — a thick layer of "reliable
  UDP for FPS-class workloads". They've converged on a 4-channel
  abstraction (unreliable / sequenced / reliable / ordered)
  that's worth knowing as a reference API.

- **DDS / RTPS.** Three actively-maintained open implementations
  (Fast-DDS, CycloneDDS, OpenDDS), one common wire protocol,
  millions of robots running it in ROS 2. Surprisingly absent
  from typical HFT design conversations even though the
  protocol is NAK-based reliable multicast over UDP.

- **Blockchain transports.** Solana Turbine in particular is the
  highest-throughput UDP-reliable broadcast on the planet right
  now and it uses FEC instead of NAK. Worth knowing.

Thinner than expected:

- **Non-Chinese, non-English-language trading protocols.** TSE
  arrowhead (Japan), KRX (Korea), MOEX (Russia), B3 (Brazil)
  are all closed and have no open-source companions even at
  the wire-format-decoder level. Some Wireshark dissectors exist
  but nothing meaningful as transport competitors.

- **Bytedance / Tencent / Alibaba trading-internal libs.**
  Tencent's tquic and Alibaba's xquic are public QUIC impls,
  but neither company has open-sourced their internal HFT
  transports (if those exist). The Chinese AppSec/network
  ecosystem has more wireless / IoT-oriented contributions
  (skywind3000/kcp, F-Stack) than HFT-oriented ones.

- **Academic / NSDI / SIGCOMM RUDP papers with usable code.**
  Plenty of papers exist (Homa, NDP, R2P2, eRPC) but most code
  is research-quality, single-author, abandoned within 18
  months of publication. None of them are production-grade
  alternatives to Aeron / Quinn / Chronicle. Cited indirectly.

- **NORM in non-C languages.** Rust crates exist (norm-rs etc.)
  but all have <5 stars. NRL's C++ impl is the only practical
  one, which is somewhat surprising given how the IETF
  standardised it 16 years ago.

- **Erlang reliable UDP.** The Erlang/OTP community went all-in
  on TCP for distributed Erlang. No notable open-source Erlang
  RUDP project beyond `gen_udp` (raw sockets). Surprising given
  Erlang's telecom DNA.

## Surprises

1. **Aeron is dominant in trading more than expected.**
   Multiple Rust ports (UnitedTraders/aeron-rs, rusteron) and
   a 2025 AWS benchmark put Aeron at ~21 µs p50 on commodity
   cloud. Real Logic was acquired by Adaptive in 2022 but the
   open project's commit cadence is healthy
   (last commit 2026-05). Citing "we are Aeron simplified to a
   single trust model" is more defensible than I assumed.

2. **The Holepunch / udx ecosystem is interesting but tiny.**
   75 stars on libudx, 27 on udx-native. It's the
   Hyperswarm / Pear Browser P2P stack and embeds reliable UDP
   plus NAT traversal. The relevant insight: even fully-P2P
   designs converge on "reliable, multiplexed, congestion-
   controlled streams over UDP", which is also what QUIC is.
   The space genuinely has one shape and many names.

3. **Solana Turbine is using FEC, not NAK, and it works at
   scale.** ~32:32 Reed-Solomon erasure coding lets validators
   reconstruct lost shreds without round-tripping back to the
   sender. This is the *opposite* of CMP's NAK approach.
   Worth a `compare/turbine.md` just to articulate the
   "proactive FEC vs reactive NAK" trade-off concretely.

4. **The Chronicle / OpenHFT ecosystem is bigger than the
   `chronicle-queue.md` doc suggests.** Chronicle FIX (FIX
   parser at <1 µs/msg), Chronicle Engine, Chronicle Network,
   Chronicle Wire, Chronicle Map. Several of these are GitHub-
   open under Apache-2.0 even though the commercial Enterprise
   tier is what most installations use. There's enough material
   for an "OpenHFT-as-a-platform" doc, not just the queue.

5. **Apache Iggy is the closest direct architectural sibling.**
   Rust, io_uring, thread-per-core, persistent log,
   sub-millisecond p99. Different topology (it's a broker;
   rsx-dxs is broker-less) but the design DNA is the same.
   Likely worth a `compare/iggy.md` for honesty.

6. **TigerBeetle's choice of VSR over RAFT is starting to look
   prescient.** Their open-sourced VSR-Zig is referenced as a
   reading group exercise inside several other projects. Their
   $20k consensus challenge produced public TLA+ specs. For
   rsx-dxs's eventual multi-replica story, VSR is now the
   more natural starting point than RAFT.

7. **Almost nobody else builds the "WAL as audit log = wire format
   = ML training data" claim.** Chronicle Queue is closest
   (WAL = memory-mapped file = wire-readable). Pulsar has tiered
   storage but it's a different shape (cold S3, hot Bookies).
   Kafka is the closest at the *operational* level (every
   committed record is durable + replayable + offset-addressable)
   but adds JSON / Avro / Schema-Registry layers that destroy the
   "no transformation" property. The rsx-dxs angle here is
   genuinely under-articulated in the existing literature.

8. **The "Aeron Archive vs WAL embedded in producer" axis is
   real.** Aeron Archive is a separate sidecar; rsx-dxs WAL is
   embedded in every producer. Other projects (DDS, BookKeeper,
   Pulsar) sit closer to the Aeron model (archive is a separate
   responsibility). rsx-dxs's embedded WAL is genuinely
   distinctive within this design space, not just a re-skin
   of an existing approach.

## Open questions

These came up during the survey and aren't answered yet:

1. **Should rsx-dxs measure itself against NORM or SRT directly?**
   Both are easy to benchmark on loopback. Doing so would close
   the "is the NAK model actually fast" question with numbers
   from a non-Aeron NAK implementation.

2. **Is there a published "exchange + WAL + FEC" design?**
   The closest is Solana Turbine, but that's for block
   propagation, not order flow. The question is whether anyone
   has tried FEC on the hot path of an exchange. If not,
   it's a possible angle for future rsx-dxs work — but the
   answer is probably "no, because RTT is 2 µs and a NAK is
   cheaper than 100% extra bandwidth".

3. **What happens when rsx-dxs wants multi-DC?** The current
   trust-boundary doc punts on it ("cross-DC peer auth is a
   future WalHeader.version extension"). Aeron's answer is
   cluster + Archive replication; TigerBeetle's is VSR;
   Pulsar/BookKeeper's is geo-replicated ledgers. The survey
   makes it clear there's no consensus pattern for HFT-class
   multi-DC reliable transport — it tends to be either
   "leader-follower TCP replay" (which is what rsx-dxs cold
   path already does) or "active-active with reconciliation"
   (which is much harder).

4. **Should the `compare/` set include a section on "encoding
   layers" (SBE, FAST, FlatBuffers, Cap'n Proto)?** These aren't
   transports but they share the design-space with rsx-dxs's
   `#[repr(C)]` records. The boring-doc answer is probably yes —
   one short doc that articulates "we deliberately don't
   schema-drive our wire format" is healthy to have on file.

5. **What does a future "extension" record-type look like in the
   WAL?** Several surveyed protocols use a version byte
   (Aeron, QUIC) or a feature-negotiation handshake (RTPS, FIXP)
   to extend their wire format without breaking older readers.
   rsx-dxs WAL already has a version byte at byte 8 (V0=legacy,
   V1=current per the memory snapshot). Worth a one-page
   "WAL extension model" note distilled from the survey.

## How this could be done better next time

- The GitHub API rate limit (60 unauthenticated req/h) ran out
  partway through the metadata pull. A `GITHUB_TOKEN` would
  bring it to 5000/h and would let the survey hit every repo
  in one pass. Note for the next census.
- WebSearch is heavy-tail on quality — the first three queries
  per topic give 80% of the signal, and the fourth+ tends to
  return blog-spam content farms. Hard-cap the per-topic
  searches.
- For deeply-niche repos (especially in non-English ecosystems),
  GitHub's search API + language filters via `gh search` would
  out-perform WebSearch. Did not get to use this here because
  `gh` was not installed in this worktree.

## File state

- `rsx-dxs/compare/niche.md` — created (~580 lines).
- `rsx-dxs/compare/README.md` — appended one section pointing
  at niche.md.
- `.ship/22-PROTOCOL-COMPARES/NICHE-SURVEY-REPORT.md` — this file.

No source code, existing compare docs, benches, or Cargo.toml
were touched.
