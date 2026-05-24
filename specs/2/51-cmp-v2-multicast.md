# casting v2 — Multicast Streaming

Status: **planned**. casting v1 is unicast (one sender → one receiver).
V2 extends the same wire format to one-to-many fan-out over UDP
multicast without a broker, without copying per-receiver.

---

## Motivation

v1 casting is a point-to-point pipe. One ME → one mktdata receiver works.
One ME → N consumers (recorder, risk, N marketdata shards, archiver,
ML feeder) requires N separate `CastSender` instances and N separate
WAL writes of the same bytes. That is O(N) syscalls and O(N) memory
pressure per frame.

UDP multicast delivers one kernel send to all subscribers on a
multicast group. The kernel replicates at the NIC, not in userspace.
For exchange fan-out (fills → all downstream consumers) this is the
right primitive.

This is the same model used by:
- **Aeron** (`aeron:udp?endpoint=...` with multicast MDC)
- **LBM / UMP** (the commercial predecessor)
- **OpenMama / Solace** (finance middleware)

---

## Wire format: no change

The 16-byte `WalHeader` is unchanged. `WalHeader.version` stays at
`V1`. The record type space is unchanged. A v2 receiver is wire-
compatible with a v1 sender that sends to a unicast address.

The only difference is the UDP destination: multicast group instead
of a unicast IP.

---

## Topology

```
Sender (ME)
    │ IP multicast group 239.1.1.0:5000
    ├──▶ Receiver A (mktdata-0)
    ├──▶ Receiver B (recorder)
    └──▶ Receiver C (ml-feeder)
         │
         │ NAK (unicast)
         └──▶ Sender repair port 5001
```

- **Data channel**: sender writes to multicast group; all receivers
  receive every frame via kernel multicast.
- **Repair channel**: each receiver sends NAKs to the sender's
  dedicated repair unicast address (not the multicast group). This
  avoids NAK implosion — receivers can't trigger each other's
  retransmit storms.
- **No status messages, no flow control**: v1 retired the
  `StatusMessage` window in `87b223e`, and v2 keeps that decision.
  Receivers do not throttle the sender; multicast amplifies the
  problem (one slow receiver freezing the whole group). Recovery
  is NAK (in-band) or TCP replication (out-of-band).

---

## NAK implosion suppression

Classic multicast NAK problem: if 100 receivers all detect a gap,
they all send a NAK simultaneously → sender receives 100 identical
retransmit requests → 100× retransmit traffic.

Fix (same as Aeron multicast, LBM, and PGM):

1. Each receiver sets a random NAK backoff timer `T ∈ [0, 20 ms]`
   on gap detection.
2. If the receiver hears a retransmit for the missing seq before `T`
   expires (because another receiver already NAK'd), it cancels its
   own NAK.
3. Only the first receiver to fire sends the NAK; the rest cancel.

Expected NAK load: O(1) per gap regardless of receiver count.

---

## Flow control

**Not in v2.** v1 retired `StatusMessage`/flow-control in
`87b223e`; v2 inherits that decision. In a multicast topology,
pacing the sender to the slowest receiver would freeze the whole
group on a single laggard — exactly the failure mode multicast is
supposed to amortise away. Receivers that fall behind drop their
multicast subscription and reconnect via the replication/TCP cold
path (same as today). The sender's clock is set by the matching
engine, not by consumers.

---

## Sender API changes

```rust
// v1 (unicast)
CastSender::new(dest_addr: SocketAddr, stream_id: u32, wal_dir: &str)

// v2 (multicast)
CastSender::new_multicast(
    mcast_group: SocketAddr,   // e.g. 239.1.1.0:5000
    repair_port: u16,          // unicast repair endpoint, same host
    stream_id: u32,
    wal_dir: &str,
    ttl: u8,                   // IP TTL for the multicast datagrams
)
```

The send path (`send`, `send_raw`, `tick`) is unchanged. Only the
underlying socket changes from a connected unicast UDP socket to a
multicast-enabled one (`IP_MULTICAST_TTL`, `IP_MULTICAST_LOOP`
disabled on loopback for production).

---

## Receiver API changes

```rust
// v1 (unicast)
CastReceiver::new(bind_addr: SocketAddr, sender_addr: SocketAddr, stream_id: u32)

// v2 (multicast)
CastReceiver::new_multicast(
    mcast_group: SocketAddr,   // group to join
    repair_addr: SocketAddr,   // sender's repair unicast endpoint
    iface: Option<Ipv4Addr>,   // bind interface; None = INADDR_ANY
    stream_id: u32,
)
```

Internally: `IP_ADD_MEMBERSHIP` on construction, `IP_DROP_MEMBERSHIP`
on drop. NAKs go to `repair_addr` (unicast), not to the multicast
group.

---

## Config additions (`CastConfig`)

```toml
[cmp]
mode = "multicast"             # or "unicast" (default, v1 behaviour)
multicast_group = "239.1.1.0:5000"
repair_port = 5001
multicast_ttl = 1              # stay within the LAN
nak_backoff_max_ms = 20        # implosion suppression window
stale_receiver_lag_segs = 2    # segments before eviction
```

---

## Cold reconnect path (unchanged)

A receiver that was evicted (or was never live) connects via
`ReplicationConsumer` (TCP + WAL replay), same as today. Once it has caught
up to within one window of the live tip, it rejoins the multicast
group and switches back to hot path. This transition is transparent
to the application: `CastReceiver::try_recv` returns records from
both sources in sequence order.

---

## What is NOT in v2

- **Encryption / auth**: still trusted LAN only. Use a VPN/IPsec
  layer if the multicast segment is not fully trusted.
- **Congestion control**: multicast is not TCP. If receivers are
  slower than the sender's rate, they get evicted and reconnect via
  replication cold path. There is no AIMD.
- **Partial fan-out / topic routing**: all receivers on a group
  receive all records. Topic filtering is the application's job.
  If you need selective delivery, run separate groups per stream.

---

## Implementation order

1. `socket2`-based multicast socket helpers (join/leave/TTL) — `rsx-cast/src/mcast.rs` (~60 LOC)
2. `CastSender::new_multicast` — thin wrapper, changes destination socket only
3. NAK backoff timer in `CastReceiver` — ~20 LOC addition to `recv_control`
4. Stale-receiver detection in `CastSender` — track per-receiver liveness
   (last NAK / connect time), evict receivers that fall behind by more than
   one WAL segment, hand them off to TCP replication
5. `compare/multicast.md` — bench: loopback multicast with 2, 4, 8 receivers
6. Config surface + docs

Estimated: ~250 LOC net new, ~30 LOC modified in existing sender/receiver
(no flow-control / per-receiver-window state to add — v1 dropped that path).
