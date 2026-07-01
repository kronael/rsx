# Niche / long-tail protocol census

This file is the long-tail companion to the curated `compare/` set
(raw-udp, tcp, kcp, quinn, aeron, chronicle-queue, lbm). Those each
have a deep doc + a guarantees table + (where feasible) a Criterion
benchmark in `benches/`. They were chosen because they are the
direct, well-known peers — every HFT/exchange architect reading
about rsx-cast has already heard of them.

This doc covers everything else worth naming: NAK/ACK reliable UDP
libraries, persistent-log-as-transport systems, kernel-bypass
stacks, FEC/multicast protocols, market-data wire formats, and
academic / dormant references. The bar is "could plausibly come up
in a design review or be cited by an outside reviewer". Each entry
is one paragraph; the goal is a cheap mental index, not a full
write-up.

Conventions
- "RUDP" = generic reliable-over-UDP transport (NAK or ACK).
- "PLOG" = persistent-log-as-transport (Kafka-family).
- "Stars" = GitHub stars at the time of writing (May 2026,
  ±10% drift acceptable). For projects without a primary GitHub
  repo (Chinese internal forks, commercial-only) the field is
  marked "n/a — closed source" or similar.
- "Last commit" = `pushed_at` from the GitHub API; "abandoned"
  means no commits in 18+ months.
- "Differs from rsx-cast" tries to be one concrete sentence, not
  a generic difference statement. If the entry is in a different
  category (e.g. multicast, not unicast), say so.

Triage rule applied
- Dropped: <10-star toy repos with no citations, single-commit
  graduate projects, dead exchange-experiment branches, anything
  that is just "QUIC server #47".
- Kept: research-only projects with associated papers, dormant
  but historically-cited projects, commercial-only protocols
  with public specs.

## Contents

1. NAK-based reliable UDP (the Aeron / casting family)
2. ACK-based / hybrid reliable UDP (the KCP / QUIC family)
3. Persistent-log-as-transport (the Chronicle / Kafka family)
4. Trading / market-data wire formats
5. DPDK / kernel-bypass / io_uring-native stacks
6. Multicast / pub-sub UDP overlays
7. Gossip / cluster membership (UDP-based)
8. Per-language reliable-UDP libraries
9. Honourable abandoned / historical
10. Shortlist: candidates that could deserve a `compare/<name>.md`

---

## 1. NAK-based reliable UDP

This is the Aeron / casting family: receiver detects gaps via sequence
discontinuity and sends a NAK; sender retransmits from a buffer.
Already covered in `aeron.md` and `lbm.md`. The long tail below.

### NORM — NACK-Oriented Reliable Multicast (RFC 5740)
- Repo: <https://github.com/USNavalResearchLaboratory/norm>
- License: NOASSERTION (NRL public-domain-style)
- Stars: ~115 · Last commit: 2026-05 · Status: active
- The NRL's reference IETF NORM implementation in C++. Standardised
  in RFC 5740 (with FEC building blocks in RFC 5401, 5510). NAK
  suppression via random backoff, parity-FEC option for proactive
  redundancy. Used in military tactical networks and as a ZeroMQ
  transport (see §6).
- Differs from rsx-cast casting: multicast-first (NAK suppression
  mandatory), FEC built in, runs over IP-layer routers. casting is
  unicast-only with no FEC and zero NAK suppression (single
  receiver per pair).

### OpenPGM — Pragmatic General Multicast (RFC 3208)
- Repo: <https://github.com/steve-o/openpgm>
- License: LGPL-2.1 · Stars: ~250 · Last commit: 2017 · Status:
  abandoned but still shipped as a Debian package
- C implementation of PGM, IETF experimental RFC 3208. PGM defines
  a NACK-and-data multicast protocol with optional parity packets;
  was promoted by Cisco / Microsoft in early 2000s. The library
  has been frozen for nearly a decade but still backs ZeroMQ's
  `pgm://` transport.
- Differs from rsx-cast casting: IP-layer multicast (requires multicast
  routing in the LAN), parity-FEC for proactive recovery, sender-
  tracked NAK responses (NCF). casting is per-pair unicast with no FEC.

### RIST — Reliable Internet Stream Transport
- Spec: <https://www.rist.tv/>, ref impl <https://code.videolan.org/rist/librist>
- License: BSD-2-Clause · Stars: ~270 (librist) · Status: active
- Open-source, open-spec NAK-based protocol from the broadcast
  industry (VideoLAN / Net Insight / SRT competitors merged
  efforts). RFC RTT-based NAK with bandwidth-throttled retransmit.
  Supports PSK + DTLS encryption.
- Differs from rsx-cast casting: optimised for video on lossy WAN
  (re-request horizon ~ seconds, FEC option), tunnel-mode framing
  over RTP. casting retransmits fixed-size records from a 4-h WAL
  with no FEC.

### SRT — Secure Reliable Transport
- Repo: <https://github.com/Haivision/srt>
- License: MPL-2.0 · Stars: 3 538 · Last commit: 2026-05 · Status: active
- Haivision's video transport, open-sourced 2017. ARQ + AES.
  Designed for unpredictable internet uplinks (broadcast-camera
  to studio); retransmit horizon limited by a configurable
  send-buffer (default ~ 1 s). Used by OBS, FFmpeg, gstreamer.
- Differs from rsx-cast casting: byte-stream not record-stream, no
  on-disk archive (retransmit horizon = send buffer in RAM), AES
  encryption mandatory in production deployments.

### RTPS / DDS (Object Management Group)
The DDS interoperability wire protocol is itself a NAK-based
reliable multicast over UDP. Three production implementations:

- **eProsima Fast-DDS** — <https://github.com/eProsima/Fast-DDS>,
  Apache-2.0, ~2 800 stars, active. Default ROS 2 middleware.
- **Eclipse Cyclone DDS** — <https://github.com/eclipse-cyclonedds/cyclonedds>,
  EPL-2.0, ~1 265 stars, active. Tier-1 ROS 2 middleware.
- **OpenDDS** — <https://github.com/OpenDDS/OpenDDS>, OCI-licensed
  (BSD-style), ~1 500 stars, active. Most mature, includes RTPS,
  TCP, and IP-multicast transports.

Differs from rsx-cast casting: full QoS layer (reliability, durability,
deadline, history depth), schema-driven (IDL or XML), discovery
via SPDP/SEDP. RTPS uses HEARTBEAT + ACKNACK rather than gap-fill
NAK; sender drives the conversation more than rsx-cast's NAK model.

### embeddedRTPS
- Repo: <https://github.com/embedded-software-laboratory/embeddedRTPS>
- License: MIT · Status: research / academic
- Tiny RTPS for microcontrollers (FreeRTOS + lwIP).
  Static-allocation, C++. Useful as proof-point that NAK reliable
  UDP fits in <50 kB.

### NORM in Rust
- Crate: `norm-rs` and similar — all <5 stars, dormant. The
  Rust ecosystem has no production NORM port; NRL C library is
  the only practical option.

---

## 2. ACK-based / hybrid reliable UDP

Sender-driven loss detection. KCP and Quinn already covered.

### UDT — UDP-based Data Transfer
- Original C++ from UIC: <https://sourceforge.net/projects/udt/>
  (2001-2014, dormant); GitHub mirrors: `whtghst1/udt`, `coditva/udt-c`.
- License: BSD · Status: abandoned (~2014)
- Sender-driven ACK + selective NAK, TCP-friendly congestion
  control aimed at bulk-data transfer over long-fat-pipes (sci
  computing, sky surveys). Influential design (~3k citations) but
  the project is dormant; modern displacement is QUIC.
- Differs from rsx-cast casting: bulk-throughput oriented (10 GbE WAN),
  large window, slow start. casting is small-message latency-oriented
  with zero congestion control.

### DCCP — Datagram Congestion Control Protocol (RFC 4340)
- Linux kernel: in-tree since 2.6.14 (2005); rarely used.
- IETF "unreliable but congestion-controlled" datagram protocol.
  Mostly displaced by QUIC's DATAGRAM frame (RFC 9221). Worth
  naming for completeness; never gained production deployment.
- Differs from rsx-cast casting: kernel-resident, no application-
  level retransmit, congestion-controlled drops rather than NAK
  recovery.

### uTP / libutp — Micro Transport Protocol
- Repos: <https://github.com/bittorrent/libutp> (frozen 2023),
  <https://github.com/transmission/libutp> (active fork)
- License: MIT · Stars: 1 154 (BT) + 8 (transmission)
- BitTorrent's reliable UDP, MIT 2010. LEDBAT congestion control
  (delay-based, scavenger). Heavily deployed (every BT client)
  but as a background-priority transport.
- Differs from rsx-cast casting: explicitly tries to *yield* to other
  traffic; casting assumes it owns the wire.

### UDX — Holepunch reliable UDP
- Repos: <https://github.com/holepunchto/libudx> (C),
  <https://github.com/holepunchto/udx-native> (Node bindings)
- License: Apache-2.0 · Stars: 75 / 27 · Last commit: 2026-05 · Status: active
- Reliable, multiplexed, congestion-controlled streams over UDP.
  Built for P2P (Hyperswarm / Pear). No handshakes, no encryption
  (assumes a Noise tunnel above it).
- Differs from rsx-cast casting: P2P NAT-traversal first, stream API
  rather than message, congestion control mandatory.

### QUIC implementations not already in `quinn.md`
- **msquic** — <https://github.com/microsoft/msquic>, MIT,
  4 702 stars, active. Cross-platform C, XDP-acceleration option.
- **lsquic** — <https://github.com/litespeedtech/lsquic>, MIT,
  1 837 stars, active. LiteSpeed's; powers their commercial WS.
- **ngtcp2** — <https://github.com/ngtcp2/ngtcp2>, MIT, 1 470,
  active. Pairs with nghttp3; pluggable TLS backend.
- **mvfst** — <https://github.com/facebook/mvfst>, MIT, 1 641,
  active. Facebook's; deployed at scale on Android/iOS, uses
  folly + fizz.
- **s2n-quic** — <https://github.com/aws/s2n-quic>, Apache-2.0,
  1 347 stars, active. AWS's, Rust, pairs with s2n-tls/rustls.
  Direct alternative to Quinn in Rust.
- **picoquic** — <https://github.com/private-octopus/picoquic>,
  MIT, 734, active. Christian Huitema's; research reference impl.
- **aioquic** — <https://github.com/aiortc/aioquic>, BSD-3, 1 982,
  active. Python; used by mitmproxy + WPT.
- **Tencent tquic** — <https://github.com/Tencent/tquic>, Apache,
  1 413 stars, active. Rust; multipath support, formally verified
  with Ivy. Bigtech-China answer to Quinn.
- **Alibaba xquic** — <https://github.com/alibaba/xquic>, Apache,
  1 890 stars, active. C; BoringSSL + BabaSSL backends.
  Targets mobile (Android/iOS/HarmonyOS).
- **Mozilla neqo** — <https://github.com/mozilla/neqo>, Apache,
  2 184 stars, active. Rust, NSS TLS. Firefox's QUIC.

Why these don't each deserve a `compare/<name>.md`: they all
implement the same RFC 9000 wire protocol. The `quinn.md`
guarantees table applies to all of them with ±10% latency
differences. If we wanted a "fastest QUIC on Linux" race, the
contenders would be msquic (XDP), mvfst (folly), and lsquic.

### QUIC DATAGRAM extension (RFC 9221)
Not a protocol but an addressable difference. RFC 9221 adds
unreliable datagrams to a QUIC connection — i.e. a "send and pray"
mode while keeping the encrypted/authenticated session. Every
mainstream QUIC impl above supports it. This is the closest
analogue to casting's per-message unreliable semantics within a
connection-oriented framing.

### Tachyon
- Repo: <https://github.com/gamemachine/tachyon-networking>
- License: ? · Status: low-activity (last commit 2024)
- Pure-Rust reliable UDP for games + IPC. NAK-based, parallelised
  by running multiple Tachyon instances pinned to different ports
  rather than internal concurrency primitives. Cited on
  gamedev.net but small community.
- Notable for being one of the few NAK-based (not ACK-based)
  Rust RUDP libraries.

### Other Rust / niche RUDP
- **rust-raknet** — <https://github.com/b23r0/rust-raknet>,
  ~225 stars. RakNet protocol in Rust; Minecraft Bedroad compat.
- **laminar** — <https://github.com/TimonPost/laminar>, 870
  stars, last commit 2023-10. Was the Amethyst engine's transport;
  Amethyst is dead, laminar effectively abandoned.
- **enet-rs** — <https://github.com/spearman/enet-rs>, FFI
  wrapper around C ENet. Old; new Rust gamedev prefers `renet`.
- **reliudp** — <https://github.com/Cobrand/reliudp>, ~50 stars.
- **uflow** — <https://github.com/lowquark/uflow>, ~40 stars.
  ACK-based, congestion-controlled (RFC 5348 TCP-friendly CC),
  64 virtual channels per connection with ordered delivery per
  channel. Aimed at internet game use. Not NAK-based; no
  persistent log or disk retransmit.
- **rudp** — <https://crates.io/crates/rudp>, ~10 stars. Thin
  state-wrapper over `UdpSocket` letting callers select per-message
  reliability (unreliable / reliable-unordered / reliable-ordered)
  at call time. Not evaluated in depth; low activity.
- **bytes-cast, reliable-udp, rkyv-net** — names suggested as
  Rust reliable-UDP candidates. Not found on crates.io as of
  May 2026. If they exist under different names, they are below
  the 10-star triage threshold.
- **naia** — <https://github.com/naia-lib/naia>, 1 127 stars,
  active. Cross-platform (incl. WASM) game networking;
  WebRTC + UDP transports under one API.

### Game-net stack (Glenn Fiedler family)
- **netcode.io** — <https://github.com/mas-bandwidth/netcode>,
  BSD-3, 2 568 stars. Connection setup + secure tokens over UDP.
- **reliable.io** — <https://github.com/mas-bandwidth/reliable>,
  BSD-3, 641 stars. Standalone ACK-based reliability layer.
- **yojimbo** — <https://github.com/mas-bandwidth/yojimbo>,
  BSD-3, 2 674 stars. Full client-server FPS stack built on the
  two above.

All three are by Glenn Fiedler (GafferOnGames). Used by Valve,
Activision, indie studios. Single-author maintained but stable
APIs since 2018. Differs from rsx-cast: client-server gaming
focus (move/shoot at 60 Hz), per-packet AEAD encryption,
serialization helpers (bit-packer). No persistent log.

### LiteNetLib / Lidgren / Photon / Mirror
.NET game-net libraries; ACK-based; reliable + sequenced channels.
LiteNetLib (3 556 stars) and Mirror are actively developed;
Lidgren-gen3 (1 218 stars) abandoned since 2021 but still
referenced. Useful as reference for "what a mainstream gaming
RUDP API looks like" — they all converged on the same channel
abstraction (Unreliable, ReliableUnordered, ReliableOrdered,
ReliableSequenced).

---

## 3. Persistent-log-as-transport

Chronicle Queue covered in `chronicle-queue.md`. This is the long
tail of "the log is the message bus", grouped by replication model.

### Apache Kafka (reference point)
Not surveyed in depth here because everyone knows it. Wire format:
TCP, length-prefixed Records framed into RecordBatch. Replication
via Fetch API (followers pull from leader). 99th-pct latency ~1
order of magnitude above Chronicle Queue (~ms vs µs). The point
of citing it: Kafka is what rsx-cast is *not*.

### Apache BookKeeper + DistributedLog + Pulsar
- BookKeeper repo: <https://github.com/apache/bookkeeper>, 1 995
  stars, active. Quorum-based replicated WAL; the substrate.
- DistributedLog: <https://github.com/twitter-archive/distributedlog>,
  2 208 stars, ARCHIVED (2020). Twitter's serving layer on top
  of BK, now subsumed into Pulsar.
- Pulsar: <https://github.com/apache/pulsar>, 15 251 stars,
  active. The current consumer of BookKeeper.
- Differs from rsx-cast: ZooKeeper coordination, quorum writes
  across N bookies, ledger lifecycle management. rsx-cast WAL is
  single-writer, no quorum, no metadata service.
- Worth citing as the "if you wanted casting-style live + WAL-style
  cold but with strong replication guarantees" reference design.

### LogDevice (Facebook, archived)
- Repo: <https://github.com/facebookarchive/LogDevice>, 1 901
  stars, archived 2021-10.
- C++ distributed log. Facebook used it for >1 TB/s of internal
  traffic. Open-sourced 2018, archived 2021 — internal
  successor not released. Citable as a published-architecture
  reference (their 2018 paper on flexible epoch placement is
  still worth reading).

### NATS JetStream
- Repo: <https://github.com/nats-io/nats-server>, 17k+ stars,
  active.
- Persistent log layer on top of NATS Core. RAFT-based replication
  optimised to reuse the data plane for consensus messages. Stream
  storage is per-server (no shared ledger). Linearizable writes.
- Differs from rsx-cast: connection-oriented client API, JSON or
  binary headers, per-stream RAFT group rather than single-writer
  WAL.

### Apache Iggy (incubating)
- Repo: <https://github.com/apache/iggy>, 4 256 stars, active.
- Rust, thread-per-core via io_uring + compio, Kafka-style log,
  TCP/QUIC/HTTP transports. Aiming for "Kafka semantics, Aeron
  performance".
- Differs from rsx-cast: still a *broker* (clients connect, then
  produce/consume). rsx-cast has no broker — every writer owns
  its own WAL.

### Redpanda
- Repo: <https://github.com/redpanda-data/redpanda>, 12 125 stars,
  active. License: BSL (not open-source by OSI).
- C++ on Seastar (thread-per-core). Kafka wire-protocol
  compatible. RAFT replication. The de-facto "fast Kafka" today.
- Differs from rsx-cast: ditto Iggy — broker model, plus Kafka API
  semantics (consumer groups, offsets, transactions).

### Raft Engine (TiKV)
- Repo: <https://github.com/tikv/raft-engine>, ~400 stars, active.
- Embedded log-structured storage for *raft logs*. Not a
  transport but a relevant data-structure reference: how to
  pack many small append-only logs into a single file efficiently.
- Worth reading next to rsx-cast WAL design (different problem —
  one shared log vs many independent logs — but same constraints
  on append-only, durable, recoverable).

### Materialize (Persist)
- Repo: <https://github.com/MaterializeInc/materialize>, ~6k stars,
  active.
- Internal "Persist" library = durable time-varying-collection
  store backed by S3 + RocksDB. Not a wire protocol but a sibling
  abstraction (durable named log). Cited for "WAL-as-source-of-
  truth" thinking.

### TigerBeetle
- Repo: <https://github.com/tigerbeetle/tigerbeetle>, 15 985 stars,
  active.
- Financial transactions DB in Zig. Uses Viewstamped Replication
  (VSR) rather than RAFT. Single-binary, no ZK / etcd. Worth
  citing for "VSR is a valid alternative to RAFT for exchange-
  class persistence" — directly relevant if rsx-cast ever needs
  multi-replica state.
- Differs from rsx-cast: VSR consensus across replicas vs casting
  unicast + WAL replication. TigerBeetle is the DB; rsx-cast is
  the transport.

### Sled / Hills (Rust embedded KV)
- sled: <https://github.com/spacejam/sled>, 8.5k+ stars.
- Log-structured embedded KV in Rust. Pre-alpha for years; cited
  here because rsx-cast WAL writer shares some design DNA
  (segment files, fsync semantics, single writer).
- Differs from rsx-cast: it's a KV index over a log, not a log
  qua transport.

### Hazelcast Jet
- Stream processor (Java) with millisecond p99.99. Not a
  transport per se. Cited only as "low-latency JVM stream
  processing" reference next to Chronicle Queue.

### Wallaroo / Pony, then Rust
- Repo: <https://github.com/WallarooLabs/wally>, ~1.4k stars.
- Distributed stream processor originally in Pony, rewritten in
  Rust 2020-ish, then the company pivoted. Project effectively
  inactive. Cited only as historical proof that "exchange-class
  stream processing in non-mainstream languages was tried".

---

## 4. Trading / market-data wire formats

Different category — these are not transports, they are
record formats. But they belong in the survey because they're
what *rides* on the transport.

### MoldUDP64 (NASDAQ)
- Spec: <https://www.nasdaq.com/docs/MoldUDP64-for-Nasdaq-Nordic-v.1.00.2.pdf>
- Reference impls: paritytrading/nassau (Java), kjx98/go-mold (Go).
- UDP-multicast protocol with sequence numbers + a separate TCP
  re-request server for gap recovery. Used for NASDAQ ITCH market
  data delivery.
- Differs from rsx-cast casting: gap recovery via a *different
  channel* (TCP rewinder service). casting gap recovery is in-band
  (NAK over the same UDP socket, retransmit from same producer).

### ITCH (NASDAQ market data)
- Format: <https://www.nasdaqtrader.com/content/technicalsupport/specifications/dataproducts/nqtvitchspecification.pdf>
- Binary record stream (per-message variable-length, byte-packed
  big-endian, ASCII alpha types). Carried over MoldUDP64.
- Open-source decoders: paritytrading/nassau, plus dozens of
  per-language parsers on GitHub.

### OUCH (NASDAQ order entry)
- Format: <https://www.nasdaqtrader.com/Trader.aspx?id=OUCH>
- Order-entry counterpart to ITCH. Binary, fixed-layout, carried
  over SoupBinTCP.

### SoupBinTCP / SoupTCP
- The TCP framing under OUCH (and many other NASDAQ-derived
  protocols). Sequence numbers, login + heartbeats. Open
  decoders in Wireshark (`packet-soupbintcp.c`).
- Differs from rsx-cast DXS/TCP replay: SoupBinTCP is a *live*
  session protocol; rsx-cast TCP is replay-only (cold path),
  live orders are UDP.

### SBE — Simple Binary Encoding
- Repo: <https://github.com/real-logic/simple-binary-encoding>,
  Real Logic, 3k+ stars, active.
- OSI L6 wire encoding. Schema → generated Java/C++/C#/Go/Rust
  decoders. Used by CME (MDP3, iLink3) and many exchanges. Pairs
  with Aeron in production.
- Differs from rsx-cast message layout: SBE is a *schema-driven*
  binary format. casting/WAL records are hand-rolled `#[repr(C)]`
  Rust structs. SBE adds versioning + tooling; casting/WAL trades
  that for one less codegen step.

### FAST — FIX Adapted for STreaming
- Spec: FIX FAST 1.1, 1.2.
- Stream-compression layer over FIX. Used historically for
  market-data multicast (squeezes correlated fields out). Modern
  CME replaced FAST with SBE.
- Open impls: objectcomputing/mFAST (C++, BSD), nathantippy/jFAST
  (Java), mcsakoff/rs-fastlib (Rust). All low-activity but stable.

### CME MDP 3.0 / iLink3
- CME's current binary market-data and order-entry protocols.
  Both use SBE encoding; MDP3 over UDP multicast, iLink3 over
  TCP with FIXP session layer.
- No FOSS spec; CME ClientSystems wiki is public. Worth citing
  as "the largest production deployment of SBE+Aeron-style".

### FIXP — FIX Performance Session Layer
- Spec: <https://www.fixtrading.org/standards/fixp-online/>
- FIX-Trading-Community's session layer for high-perf binary
  trading. Origins: NASDAQ SoupBinTCP + UFO ("UDP for Orders").
  Variants over TCP and UDP. UFO is the closest spec analogue
  to rsx-cast casting (sequenced order datagrams with NAK recovery).
- Differs from rsx-cast casting: standardised session-layer
  bracketing (Negotiate, Establish, Terminate), per-flow auth,
  designed for client-vendor interop. casting has no session
  bracketing (sendto on first frame, NAK on first gap).

### QuickFIX (open-source FIX engine)
- Repos: quickfix/quickfix (C++), quickfix-j/quickfixj (Java),
  quickfixgo/quickfix (Go), connamara/quickfixn (.NET).
- The de-facto open FOSS FIX engine since 2001. Carries FIX 4.x
  / 5.x messages over TCP. Stars range ~1-2.5k per impl. All
  active.
- Differs from rsx-cast: tag/value text protocol, TCP-only, no
  WAL semantics (relies on session-level msg numbers + a store).

### Project Parity (paritytrading)
- Repo: <https://github.com/paritytrading/parity>, ~600 stars.
- Open-source JVM trading system: matching engine + FIX gateway
  + ITCH/OUCH publishers + terminal client. Maintained by a
  small team since 2014.
- Cited as the open reference for "what an exchange built around
  NASDAQ wire formats looks like in Java".

### OpenMAMA
- Repo: <https://github.com/finos/OpenMAMA>, ~330 stars, active.
- Vendor-neutral market-data middleware API. Wraps Solace,
  Tibco RV, ZeroMQ, etc. Originally Wombat → NYSE → Linux
  Foundation → FINOS. C/C++/Java/C#.
- Differs from rsx-cast: API layer, not a wire protocol.
  Useful as "what does a polyglot market-data API look like".

### Liquibook
- Repo: <https://github.com/enewhuis/liquibook>, ~1k stars,
  low-activity.
- C++ matching-engine *components* (order book, BBO tracking).
  Not a transport. Cited for completeness — what an open ME
  looks like at the algorithm level.

### Solana Turbine
- Spec: <https://docs.anza.xyz/consensus/turbine-block-propagation>
- Used by every Solana validator (agave, firedancer, jito-solana).
- UDP-based reliable block-propagation for a blockchain.
  Block split into ~1200-byte "shreds"; Reed-Solomon 32:32
  erasure coding (FEC); recipients form a layered tree
  (Turbine). Recently QUIC has been overlaid on the shred path.
- Differs from rsx-cast casting: FEC instead of NAK (proactive rather
  than reactive), tree multicast topology, slot-bounded retention
  (you only retransmit within a block). casting is per-pair unicast,
  no FEC, 4-h retention.
- Citable as the most-deployed UDP-reliable transport on
  the planet by raw bandwidth (Solana mainnet sustains
  ~tens of Gbps per validator).

### Firedancer (Jump Crypto)
- Repo: <https://github.com/firedancer-io/firedancer>, ~6k stars,
  active.
- Independent Solana validator in C, kernel-bypass (DPDK + AF_XDP)
  + io_uring. Implements Turbine + a custom QUIC + ed25519 batch
  verification. Sub-Frankendancer hybrid was the 2025 production
  variant.
- Cited as the current ceiling for "userspace networking + log
  reconstruction" in production.

---

## 5. DPDK / kernel-bypass / io_uring-native stacks

Not transports; they are *under* the transport. Cited because
rsx-cast's "later: DPDK/AF_XDP swap" line implies this design space.

### Seastar
- Repo: <https://github.com/scylladb/seastar>, 9 228 stars, active.
- C++ thread-per-core framework. Used by ScyllaDB, Redpanda,
  Ceph Crimson. Includes its own userspace TCP/IP over DPDK.
- Differs from rsx-cast: framework, not a library. You write code
  *in* Seastar's future/promise style. rsx-cast hot path is
  threads + SPSC rings, no future combinators.

### Glommio
- Repo: <https://github.com/DataDog/glommio>, 3 590 stars, active.
- Rust thread-per-core on io_uring. Datadog. Three rings per
  thread (poll for NVMe, submission/completion). Closer
  Rust-native equivalent of Seastar.
- Differs from rsx-cast: async/await everywhere; rsx-cast gateway
  uses monoio (also io_uring), risk/ME use plain tokio + SPSC
  for hot path.

### F-Stack
- Repo: <https://github.com/F-Stack/f-stack>, 4 219 stars, active.
- Tencent. FreeBSD-port userspace TCP/IP over DPDK + coroutine
  API + Nginx/Redis ports. Aimed at web-scale, not HFT.

### mTCP
- Repo: <https://github.com/mtcp-stack/mtcp>, 2 126 stars,
  last commit 2024-07.
- Academic (KAIST), the early-2010s reference for "userspace
  TCP on DPDK". Less maintained than F-Stack now.

### smoltcp
- Repo: <https://github.com/smoltcp-rs/smoltcp>, 4 462 stars,
  active.
- Rust embedded TCP/IP stack, no heap allocation. Targeted at
  bare-metal / RTOS. Cited as "is rsx-cast ever going to want a
  Rust-native userspace stack?" — answer is probably no (smoltcp
  trades throughput for embeddedness; we want the opposite).

### Snabb
- Repo: <https://github.com/snabbco/snabb>, 3 032 stars,
  last commit 2024-08 (slow but not dead).
- LuaJIT user-space packet-processing toolkit. NFV oriented.
  Cited only as "another userspace networking stack you'll
  encounter in the literature".

### VPP / VCL (FD.io)
- Vector Packet Processing + the VCL "communications library"
  socket-API shim. FD.io project (Cisco origins). Big iron,
  carrier-grade. Aeron has been ported to run on VPP's host stack.

### OpenOnload + ef_vi + TCPDirect (AMD / Solarflare)
- onload: <https://github.com/Xilinx-CNS/onload>, 805 stars,
  active. LD_PRELOAD-able TCP/IP stack on Solarflare NICs.
- tcpdirect: <https://github.com/Xilinx-CNS/tcpdirect>, 80 stars,
  active. Lower-latency proprietary API. ~20 ns half-RTT claim.
- ef_vi: the lowest layer (raw Ethernet frame queues).
- These are *not* transports but enable rsx-cast to run faster
  on Solarflare HW (which is the standard HFT NIC). Cited
  because every HFT design conversation hits these names.

### Mellanox VMA / DPDK
- VMA (Voltaire Messaging Accelerator, now NVIDIA-branded).
  LD_PRELOAD UDP accelerator for ConnectX/ConnectX-5 NICs.
  Mostly displaced by DPDK and AF_XDP in green-field builds.

### Iceoryx + iceoryx2
- iceoryx: <https://github.com/eclipse-iceoryx/iceoryx>, 2 071
  stars. C++ true zero-copy IPC for AUTOSAR / ROS 2.
- iceoryx2: <https://github.com/eclipse-iceoryx/iceoryx2>, 2 267
  stars, active. Rust rewrite. POSIX shared memory, no broker.
- Differs from rsx-cast SPSC: cross-process IPC (rsx-cast SPSC
  is intra-process), publish/subscribe with multiple consumers,
  larger payloads. iceoryx2 is the cleanest open Rust alternative
  to rtrb for cross-process zero-copy.

### Eclipse Zenoh
- Repo: <https://github.com/eclipse-zenoh/zenoh>, 2 784 stars,
  active.
- Rust. Pub/sub + query + storage. Min wire overhead 4 B.
  Has a zenoh-pico for microcontrollers. ROS 2 RMW.
- Differs from rsx-cast: more like a fabric than a transport
  (peer-to-peer routing, no fixed topology, schema-less).

---

## 6. Multicast / pub-sub UDP overlays

### ZeroMQ (libzmq)
- Repo: <https://github.com/zeromq/libzmq>
- License: MPLv2 · Stars: 10k+ · Active.
- Brokerless messaging library. UDP/multicast transports via
  PGM (`zmq_pgm`), EPGM (PGM tunneled in UDP), and NORM
  (`zmq_norm`). Other transports: TCP, IPC, inproc, WebSocket.
- Differs from rsx-cast: high-level socket abstractions
  (REQ/REP, PUB/SUB, PUSH/PULL etc.) — rsx-cast casting is a single
  per-pair unicast pipe.

### nanomsg-next-generation (nng)
- Repo: <https://github.com/nanomsg/nng>, 4 586 stars, active.
- ZeroMQ successor by Garrett D'Amore. Same scalability protocols
  (PUB/SUB, REQ/REP …), better transport abstraction (TLS, WS,
  ZeroTier transports added relatively easily). No PGM by default;
  community has discussed UDP reliable transports (see "UDP Design
  Notes" wiki) but none merged.

### TIBCO Rendezvous (RV / RVD)
- Commercial; widely deployed in trading (TIBCO's biggest hit).
- UDP-multicast with TCP fallback. NAK-based with a configurable
  "reliability interval". RVD daemon model.
- Cited as the historical 1990s-2000s trading multicast bus.
  Largely displaced by 29West LBM / Solace / Aeron.

### Solace / IBM MQ Low Latency Messaging
- Commercial appliance + software. Cited as the commercial peer
  of LBM that doesn't have a `compare/` doc.

### Multi-Destination-Cast (Aeron MDC)
- Not a separate library; an Aeron feature. UDP unicast that
  flow-controls like multicast (each receiver tracked
  individually). Useful when L3 multicast isn't available
  (most clouds).
- Cited because "Aeron without multicast" is closer to rsx-cast
  casting than "Aeron with multicast" — both are per-pair unicast.

---

## 7. Gossip / cluster membership over UDP

Not in scope as a transport competitor, but worth cataloguing
because rsx-cast may later need failure detection.

### hashicorp/memberlist
- Repo: <https://github.com/hashicorp/memberlist>, 4 061 stars,
  active.
- Go. SWIM-based. UDP gossip + periodic TCP full state sync.
  Used by Consul, Serf, Nomad.

### HyParView + Plumtree
- Reference: Leitão et al. 2007 papers.
- HyParView = membership protocol (two partial views, robust to
  churn). Plumtree = eager-tree + lazy-gossip broadcast on top.
- Rust impls: <https://github.com/sile/hyparview> (research),
  libp2p-episub. No production-grade impl yet.

### SWIM (Scalable Weakly-consistent Infection-style Process
Group Membership)
- The paper that started the gossip-failure-detection family.
  memberlist is the closest production impl.

---

## 8. Per-language reliable-UDP libraries

A quick census, beyond the Rust/.NET/Java already covered.

### C / C++
- ENet — old, lots of forks, gaming-focused.
- ZeroTier ZTNetkit — closed-source-ish but ZT itself is GPL.
- usrsctp — userland SCTP, broadly used in WebRTC stacks.

### Go
- pion/sctp — pure-Go SCTP, used by pion-WebRTC.
- ishidawataru/sctp — kernel SCTP wrapper.
- nats-io/nats-server — JetStream wire is TCP, but worth
  knowing as a reference.

### Java / JVM
- Agrona — <https://github.com/aeron-io/agrona>, 1 000+ stars.
  Real Logic's low-allocation data-structure library
  (ring buffers, off-heap maps). Companion to Aeron + SBE.
- LMAX Disruptor — <https://github.com/LMAX-Exchange/disruptor>,
  18 344 stars. The ring-buffer design that started it all.
  Not a transport but the pattern lives in rsx-cast's SPSC rings.

### Erlang/Elixir
- gen_udp (OTP kernel) — just sockets, no reliability.
- Distributed Erlang uses TCP by default; UDP mode exists but
  is unusual.

### Python
- aioquic (covered in §2).
- asyncio_udp — stdlib, no reliability layer.

### Crystal / Nim / Zig
- No notable reliable-UDP libraries discovered. The Zig
  ecosystem has TigerBeetle's VSR but that's a database, not
  a generic transport.

---

## 9. Honourable abandoned / historical

Projects that influenced the design space but aren't actively
maintained. Worth knowing the names; not worth depending on.

| Project | Era | Lineage | Status |
|---|---|---|---|
| UDT | 2001-2014 | UIC bulk-data RUDP | abandoned, ~3k citations |
| OpenPGM (libpgm) | 2006-2017 | RFC 3208 ref | maintenance only, still Debian-packaged |
| Lidgren.Network gen3 | 2010-2021 | .NET game RUDP | dead 2021, forks live on |
| RakNet | 2003-2014 | C++ gaming RUDP | dead, MMO codebases use forks |
| TIBCO RV | 1990s-now | trading multicast | commercial, replaced by LBM/Aeron in new builds |
| DCCP | 2005-?? | RFC 4340 kernel proto | in Linux but no deployment |
| LogDevice | 2014-2021 | FB distributed log | open-sourced then archived |
| DistributedLog | 2013-2020 | Twitter, → Pulsar | archived; the spirit lives in Pulsar |
| Wallaroo / Pony | 2017-2020 | streaming in Pony | rewritten in Rust then EOL |
| Snabb | 2012-2024 | LuaJIT userspace | slow, possibly dormant |
| mTCP | 2014-2024 | DPDK TCP research | slow, possibly dormant |
| FAST (FIX) | 2005-2015 | FIX market-data | superseded by SBE |
| MAMA / Wombat | 1998-2010s | MD middleware | now OpenMAMA / FINOS |
| Aeron / Real Logic (pre-Adaptive) | 2014-2022 | the namesake | acquired 2022, but project itself active |

### Things you'll see in old papers but should not implement
- Reliable Multicast Transport Protocol (RMTP, RFC 3208 era).
- Scalable Reliable Multicast (SRM, Floyd et al. 1995).
- LBT / Tibco SmartSockets pre-RV.

These predate modern NAK-based unicast practice and are mostly
historical curiosities for someone tracing the multicast → unicast
transition that LMAX/Aeron embodied.

---

## 10. Shortlist — promote to `compare/<name>.md`?

Five-to-ten candidates that, after this survey, look like they'd
genuinely earn their own deep doc + guarantees table.

### Strong yes (4)

1. **SRT** — `compare/srt.md`. Different niche (video, lossy WAN)
   but the *only* mainstream NAK-based reliable UDP with a
   straightforward C API and an active community. Useful "this
   is what NAK reliable UDP looks like outside HFT" foil.
   Existing repo is BSD-licensed enough to vendor or link
   against. *Benchmark feasibility: yes — librist/libsrt are
   easy to build and have C examples; can run loopback.*

2. **eProsima Fast-DDS** — `compare/dds.md` (covers RTPS family).
   Single doc that explains RTPS + HEARTBEAT/ACKNACK and stands
   in for Fast-DDS / CycloneDDS / OpenDDS (they share the wire
   protocol). DDS is the *most* widespread NAK-based UDP system
   by deployment count (every ROS 2 robot in the world). The
   QoS layer also gives a good "what rsx-cast doesn't do and why"
   contrast. *Benchmark feasibility: medium — Fast-DDS pubsub
   loopback is straightforward but builds drag in TinyXML +
   FastCDR.*

3. **Solana Turbine** — `compare/turbine.md`. The
   highest-throughput UDP reliable broadcast in production.
   FEC-based (not NAK), tree multicast, slot-bounded retention.
   Direct foil to rsx-cast's NAK + WAL model. The Firedancer
   implementation is in C and is benchmark-able. *Benchmark
   feasibility: low — full Turbine requires a live cluster;
   we'd only quote published numbers.*

4. **NORM (RFC 5740, NRL)** — `compare/norm.md`. The IETF
   standard equivalent of casting's NAK model, but multicast and
   with FEC. NRL impl is small (~25k LOC C++). Citable as
   "what an IETF-standard casting-equivalent looks like" — would
   nicely fill the gap that exists today between `aeron.md`
   (proprietary protocol, JVM driver) and `lbm.md` (commercial).
   *Benchmark feasibility: yes — `norm` builds out of the box,
   has C examples; unicast NORM is loopback-friendly.*

### Yes-if-time (3)

5. **TigerBeetle (VSR)** — `compare/tigerbeetle.md`. Not a UDP
   transport but a sibling design: deterministic, single-binary,
   exchange-class, log-based. The "what if rsx-cast needed multi-
   replica state" question lands here. VSR vs RAFT discussion
   is independently valuable. *Benchmark feasibility: medium —
   `tigerbeetle benchmark` exists but doesn't isolate transport.*

6. **netcode.io + reliable.io + yojimbo** — `compare/gamenet.md`
   (single doc covering Glenn Fiedler's stack). It's the most
   widely cited "secure reliable UDP for games" stack and answers
   "why not just use the gaming community's RUDP?". Differs from
   rsx-cast in encryption-mandatory + per-packet AEAD + connection
   tokens. *Benchmark feasibility: yes — reliable.io has a
   loopback example, ~300 LOC to wrap in Criterion.*

7. **iceoryx2** — `compare/iceoryx.md`. Not UDP at all but the
   closest *Rust* zero-copy cross-process alternative. Useful
   for "rsx-cast SPSC is intra-process; what if you need IPC
   across processes without going to UDP?". *Benchmark
   feasibility: yes — iceoryx2 has Criterion benches upstream.*

### Maybe / lower priority (3)

8. **MoldUDP64 + SoupBinTCP** — `compare/mold-soup.md`. Combined
   doc on NASDAQ's wire format pair. Useful for showing rsx-cast
   solving the same problem in-band (NAK over the same socket)
   that NASDAQ solves by deploying a separate TCP rewinder server.

9. **SBE** — `compare/sbe.md`. Just the encoding layer, not a
   transport — but rsx-cast explicitly does *not* use SBE
   (hand-rolled `#[repr(C)]` instead), and the contrast is
   probably worth a short doc.

10. **Apache Iggy** — `compare/iggy.md`. The most direct
    architectural peer (Rust, io_uring, thread-per-core,
    persistent log). Different topology (broker vs broker-less)
    but covers similar performance claims. *Benchmark
    feasibility: high — Iggy ships a `iggy bench` tool.*

### Drop / no value-add (worth naming explicitly)

The following are mentioned above but don't deserve their own
`compare/` doc because they'd be redundant with the curated set:

- All other QUIC implementations (msquic, mvfst, picoquic,
  s2n-quic, lsquic, neqo, tquic, xquic, aioquic) — collapse
  into the existing `quinn.md`.
- ENet / RakNet / LiteNetLib / Lidgren — gaming RUDPs,
  superseded by the Fiedler stack (which itself would only
  rate one doc).
- UDT — abandoned, design ideas superseded by QUIC.
- Snabb / F-Stack / VPP — kernel-bypass stacks, not transports.
- Wallaroo / DistributedLog / LogDevice — archived.

---

## Survey scope & honest gaps

- **Not investigated in depth**: Chinese ecosystem trading
  protocols (Shanghai/Shenzhen exchange wire formats are
  closed). Mostly visible only through commercial market-data
  vendors. Bytedance has internal projects but nothing
  open-sourced as a transport library so far.
- **Japanese**: TSE arrowhead protocol and related are closed.
  No notable open-source Japanese reliable-UDP project found
  that isn't a fork of an existing one.
- **Russian / Yandex**: YDB is open but it's a database; the
  internal transport is gRPC-over-TCP. No reliable-UDP project
  spotted.
- **Korean (HYDRA / KIS / etc.)**: KIS has open APIs but they
  are REST/WebSocket. KRX wire formats closed.
- **Academic / NSDI / SIGCOMM**: a number of "reliable UDP for
  X" papers (Homa, NDP, R2P2) exist but lack a production-
  ready open implementation. Cited indirectly through their
  modern descendants (Aeron / QUIC / Turbine).

If a future audit reveals a missed project in the trading
or NAK-reliable-UDP category, it likely lives in one of:
proprietary vendor SDKs (OnixS, ToTrade, Nordic Capital), HFT
firms' internal stacks (XTX, Jane Street, Citadel — none
open-sourced), or extremely-low-star research repos that
slipped past the triage filter.
