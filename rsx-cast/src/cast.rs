//! `CastSender` + `CastReceiver`: the casting (live UDP) half. See `specs/4-cast.md`.

use crate::config::CastConfig;
use crate::encode_utils::as_bytes;
use crate::encode_utils::compute_crc32;
use crate::encode_utils::decode_payload;
use crate::header::WalHeader;
use crate::records::CastHeartbeat;
use crate::records::CastRecord;
use crate::records::Nak;
use crate::records::RECORD_HEARTBEAT;
use crate::records::RECORD_NAK;
use crate::wal::extract_seq;
use crate::wal::read_record_at_seq;
use crate::wal::Framed;
use socket2::Domain;
use socket2::Protocol;
use socket2::Socket;
use socket2::Type;
use std::io;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

/// Bind a non-blocking UDP socket with an 8 MB recv buffer.
///
/// Deliberately does NOT set SO_REUSEADDR / SO_REUSEPORT: an
/// exchange port has exactly one owner; SO_REUSEPORT would
/// load-balance datagrams across multiple binders and
/// silently shred the stream. If a dead parent is still
/// holding the port, that's a supervisor / system-level
/// problem (kill the stuck PID; configure the unit file to
/// fail fast), not something the transport should paper
/// over by allowing co-bind.
fn bind_udp(addr: SocketAddr) -> io::Result<UdpSocket> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_nonblocking(true)?;
    // Request 8 MB recv buffer. Linux silently clips to
    // /proc/sys/net/core/rmem_max (commonly 208 KB) — that's
    // fine; the call only fails on a broken socket. Larger
    // buffer reduces UDP loopback drop under burst (50k msg
    // in <500ms outpaces the default rmem on stock kernels).
    if let Err(e) = socket.set_recv_buffer_size(8 * 1024 * 1024) {
        tracing::debug!(?e, "set_recv_buffer_size failed");
    }
    socket.bind(&addr.into())?;
    Ok(socket.into())
}

/// Wire payload ceiling. `WalHeader::len` is u16, so this is
/// the protocol-level maximum — but the receiver enforces it
/// explicitly to give downstream consumers a hard upper bound
/// before any unsafe casts. Live records are <= 64 bytes
/// (one cache line); this ceiling exists to allow future
/// snapshot-style records without a wire-format change.
pub const MAX_PAYLOAD: usize = 65535;

/// UDP send/recv buffer = header(16) + max payload(65535)
const PACKET_BUF_SIZE: usize = WalHeader::SIZE + MAX_PAYLOAD + 1;

/// Preallocated send-ring frame size. Covers the 16-byte
/// header plus any current `#[repr(C, align(64))]` record
/// (all <= 64 bytes payload) with headroom. Records larger
/// than this skip the ring; NAK fallback reads them from
/// the WAL via `read_record_at_seq`.
const SEND_RING_FRAME_BYTES: usize = 128;

/// Send-ring capacity. Power of two so seq -> slot is a
/// bitwise AND. Drives the recovery horizon for NAK on the
/// hot tier; older NAKs fall through to WAL random-access.
const SEND_RING_CAPACITY: usize = 4096;
const SEND_RING_MASK: u64 =
    SEND_RING_CAPACITY as u64 - 1;

/// Receiver reorder-ring capacity. Power of two so
/// seq -> slot is a bitwise AND. Sizes the in-flight gap
/// window the receiver tolerates before transitioning to
/// FAULTED. 2048 is ~200 ms at 10 k pps or 2 s at 1 k pps —
/// comfortable margin above realistic LAN hiccups; bigger
/// gaps are recovered via DXS/TCP replay anyway.
const REORDER_CAPACITY: usize = 2048;
const REORDER_MASK: u64 =
    REORDER_CAPACITY as u64 - 1;
/// Per-slot frame size. Sized to hold every current CMP
/// record (FillRecord, BboRecord, OrderAcceptedRecord and
/// CaughtUpRecord are 128 B payload; everything else is
/// 64 B). 16 B header + 128 B payload + headroom = 256.
/// Memory cost: 2048 * 256 = 512 KB per receiver.
const REORDER_FRAME_BYTES: usize = 256;

/// Frame a record (header + payload) into `buf` and send it.
/// Returns the total wire length (`WalHeader::SIZE + payload.len()`)
/// so the caller can read back the framed bytes (e.g. send-ring cache).
fn frame_and_send(
    socket: &UdpSocket,
    buf: &mut [u8],
    record_type: u16,
    payload: &[u8],
    dest: SocketAddr,
) -> io::Result<usize> {
    let crc = compute_crc32(payload);
    let header = WalHeader::new(
        record_type,
        payload.len() as u16,
        crc,
    );
    let total = WalHeader::SIZE + payload.len();
    buf[..WalHeader::SIZE].copy_from_slice(header.to_bytes());
    buf[WalHeader::SIZE..total].copy_from_slice(payload);
    socket.send_to(&buf[..total], dest)?;
    Ok(total)
}

pub struct CastSender {
    socket: UdpSocket,
    dest: SocketAddr,
    next_seq: u64,
    last_heartbeat: Instant,
    heartbeat_interval: Duration,
    /// Preallocated retransmit cache. Three parallel arrays
    /// of length SEND_RING_CAPACITY indexed by
    /// `seq & SEND_RING_MASK`. `ring_seqs[i] == 0` means the
    /// slot has never been written; otherwise it holds the
    /// frame for that exact seq. NAK lookup checks the seq
    /// matches before re-sending. Records that don't fit in
    /// SEND_RING_FRAME_BYTES bypass the ring (ring_lens=0)
    /// and force NAK to fall through to WAL.
    ///
    /// One-time allocation at construction; **zero heap
    /// allocations on the hot send path**.
    ring_seqs: Box<[u64]>,
    ring_lens: Box<[u16]>,
    ring_frames: Box<[u8]>,
    /// Per-slot last-retransmit timestamp (ns since
    /// `start_instant`). Used to dedup NAK storms: a NAK
    /// arriving for `seq` whose slot was retransmitted
    /// within `retx_dedup_window_ns` is dropped. `0` means
    /// "never retransmitted". Parallel to `ring_seqs`.
    ring_last_retx_ns: Box<[u64]>,
    /// Monotonic clock baseline. Subtracted from
    /// `Instant::now()` to produce the u64 ns values stored
    /// in `ring_last_retx_ns`. One per sender lifetime.
    start_instant: Instant,
    /// Retransmit dedup window, in ns. Cached from config so
    /// the hot path avoids the u64 multiply on every NAK.
    retx_dedup_window_ns: u64,
    /// Stream id used by the WAL filename layout. Needed
    /// for NAK retransmit when the requested seq has
    /// fallen out of `ring_seqs`.
    stream_id: u32,
    /// Hot WAL directory; used to recover NAK targets that
    /// missed the in-memory ring.
    wal_dir: PathBuf,
    buf: [u8; PACKET_BUF_SIZE],
}

impl CastSender {
    pub fn new(
        dest: SocketAddr,
        stream_id: u32,
        wal_dir: &std::path::Path,
    ) -> io::Result<Self> {
        Self::with_config(
            dest,
            stream_id,
            wal_dir,
            &CastConfig::default(),
        )
    }

    pub fn with_config(
        dest: SocketAddr,
        stream_id: u32,
        wal_dir: &std::path::Path,
        config: &CastConfig,
    ) -> io::Result<Self> {
        let bind_str = config
            .sender_bind_addr
            .as_deref()
            .unwrap_or("0.0.0.0:0");
        let bind: SocketAddr = bind_str.parse().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid sender_bind_addr {bind_str}: {e}"),
            )
        })?;
        let socket = bind_udp(bind)?;
        Ok(Self {
            socket,
            dest,
            next_seq: 1,
            last_heartbeat: Instant::now(),
            heartbeat_interval: Duration::from_millis(
                config.heartbeat_interval_ms,
            ),
            ring_seqs: vec![0u64; SEND_RING_CAPACITY]
                .into_boxed_slice(),
            ring_lens: vec![0u16; SEND_RING_CAPACITY]
                .into_boxed_slice(),
            ring_frames: vec![
                0u8;
                SEND_RING_CAPACITY
                    * SEND_RING_FRAME_BYTES
            ]
            .into_boxed_slice(),
            ring_last_retx_ns: vec![
                0u64;
                SEND_RING_CAPACITY
            ]
            .into_boxed_slice(),
            start_instant: Instant::now(),
            retx_dedup_window_ns: config
                .retx_dedup_window_us
                .saturating_mul(1000),
            stream_id,
            wal_dir: wal_dir.to_path_buf(),
            buf: [0u8; PACKET_BUF_SIZE],
        })
    }

    /// Send a typed CMP record. Assigns seq via
    /// CastRecord::set_seq.
    ///
    /// No flow control: CMP has no backpressure. If the
    /// receiver can't keep up, recovery is via NAK (small
    /// gaps) or DXS replay (large gaps), not by stalling
    /// the producer.
    ///
    /// Hot send path: zero heap allocations. For small records
    /// (≤ SEND_RING_FRAME_BYTES) the frame is written directly
    /// into the ring slot, then sent from there — single
    /// intermediate user-space copy, no self.buf on the hot
    /// path.
    pub fn send<T: CastRecord>(
        &mut self,
        record: &mut T,
    ) -> io::Result<()> {
        let seq = self.next_seq;
        record.set_seq(seq);

        let payload = as_bytes(record);
        let total = WalHeader::SIZE + payload.len();
        let crc = compute_crc32(payload);
        let header = WalHeader::new(
            T::record_type(),
            payload.len() as u16,
            crc,
        );

        if total <= SEND_RING_FRAME_BYTES {
            // Write directly into the ring slot; send from
            // there. Populates the NAK cache in the same
            // pass — no extra buf→ring copy.
            let slot = (seq & SEND_RING_MASK) as usize;
            let off = slot * SEND_RING_FRAME_BYTES;
            self.ring_frames[off..off + WalHeader::SIZE]
                .copy_from_slice(header.to_bytes());
            self.ring_frames
                [off + WalHeader::SIZE..off + total]
                .copy_from_slice(payload);
            self.ring_seqs[slot] = seq;
            self.ring_lens[slot] = total as u16;
            self.socket.send_to(
                &self.ring_frames[off..off + total],
                self.dest,
            )?;
        } else {
            // Large record: fall back to self.buf; mark
            // slot dirty so NAK falls through to WAL.
            let slot = (seq & SEND_RING_MASK) as usize;
            self.ring_seqs[slot] = 0;
            self.ring_lens[slot] = 0;
            self.buf[..WalHeader::SIZE]
                .copy_from_slice(header.to_bytes());
            self.buf[WalHeader::SIZE..total]
                .copy_from_slice(payload);
            self.socket
                .send_to(&self.buf[..total], self.dest)?;
        }

        self.next_seq += 1;
        // Data send doubles as a liveness signal: the receiver
        // sees seq via the data record, no separate heartbeat
        // needed. Reset the timer; tick() will skip until the
        // stream goes idle.
        self.last_heartbeat = Instant::now();
        Ok(())
    }

    /// Publish a record framed by `WalWriter::prepare`. No CRC
    /// compute, no seq assignment — those costs were paid in
    /// `prepare`, and the seq from `framed.seq` is what
    /// indexes the send ring. The sender's own `next_seq`
    /// counter advances to stay in lockstep with the WAL.
    ///
    /// Paired callers (those that both persist AND publish the
    /// same record) MUST use this entry point. See
    /// `notes/crc.md` and the rsx-cast crate docs.
    pub fn send_framed(
        &mut self,
        framed: &Framed,
    ) -> io::Result<()> {
        let seq = framed.seq;
        let total = framed.total as usize;
        let wire = &framed.wire[..total];

        if total <= SEND_RING_FRAME_BYTES {
            // Single copy: framed.wire → ring slot; send from
            // there. The copy in prepare() already paid for
            // header+payload packing.
            let slot = (seq & SEND_RING_MASK) as usize;
            let off = slot * SEND_RING_FRAME_BYTES;
            self.ring_frames[off..off + total]
                .copy_from_slice(wire);
            self.ring_seqs[slot] = seq;
            self.ring_lens[slot] = total as u16;
            self.socket.send_to(
                &self.ring_frames[off..off + total],
                self.dest,
            )?;
        } else {
            // Large record: send directly from framed.wire;
            // mark ring slot dirty (NAK falls to WAL).
            let slot = (seq & SEND_RING_MASK) as usize;
            self.ring_seqs[slot] = 0;
            self.ring_lens[slot] = 0;
            self.socket.send_to(wire, self.dest)?;
        }

        // Keep sender's counter in lockstep with the WAL.
        // If the caller is paired, framed.seq == self.next_seq;
        // if seqs ever diverge, this preserves the WAL as
        // canonical and rebases the sender on top of it.
        self.next_seq = seq + 1;
        self.last_heartbeat = Instant::now();
        Ok(())
    }

    pub fn tick(&mut self) -> io::Result<()> {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat)
            >= self.heartbeat_interval
        {
            let hb = CastHeartbeat {
                highest_seq: self.next_seq
                    .saturating_sub(1),
                _pad1: [0u8; 56],
            };
            frame_and_send(
                &self.socket,
                &mut self.buf,
                RECORD_HEARTBEAT,
                as_bytes(&hb),
                self.dest,
            )?;
            self.last_heartbeat = now;
        }
        Ok(())
    }

    pub(crate) fn handle_nak(&mut self, nak: &Nak) {
        // Clamp count so a malicious or buggy peer can't
        // make us loop on u64::MAX. Beyond ring capacity
        // we'd be reading WAL anyway; cap at capacity.
        let count = nak.count.min(SEND_RING_CAPACITY as u64);
        if count != nak.count {
            warn!(
                "nak count={} clamped to {}",
                nak.count, count
            );
        }
        let now_ns =
            self.start_instant.elapsed().as_nanos() as u64;
        for i in 0..count {
            let seq = nak.from_seq.saturating_add(i);
            // Dedup: if this slot was retransmitted within
            // the dedup window, skip. Receiver's own retry
            // cadence is bounded by `nak_debounce_us` so
            // legitimate retries fall through.
            let slot = (seq & SEND_RING_MASK) as usize;
            let last = self.ring_last_retx_ns[slot];
            if last != 0
                && now_ns.saturating_sub(last)
                    < self.retx_dedup_window_ns
            {
                continue;
            }
            // Hot tier: preallocated ring lookup. Slot may
            // hold either this seq (cache hit), an older
            // seq the ring has wrapped past (cache miss
            // via seq mismatch), or be unused
            // (ring_seqs[slot] == 0).
            if seq != 0
                && self.ring_seqs[slot] == seq
                && self.ring_lens[slot] > 0
            {
                let len = self.ring_lens[slot] as usize;
                let off = slot * SEND_RING_FRAME_BYTES;
                if let Err(e) = self.socket.send_to(
                    &self.ring_frames[off..off + len],
                    self.dest,
                ) {
                    warn!(
                        "nak retransmit send \
                         failed seq={seq}: {e}"
                    );
                } else {
                    self.ring_last_retx_ns[slot] = now_ns;
                }
                continue;
            }
            // Ring miss — fall back to disk. The WAL has
            // every seq we've ever appended (until GC), so
            // NAK retransmit works for records older than
            // send_ring_limit.
            match read_record_at_seq(
                self.stream_id,
                seq,
                &self.wal_dir,
            ) {
                Ok(Some(rec)) => {
                    let total = WalHeader::SIZE
                        + rec.payload.len();
                    if total > self.buf.len() {
                        warn!(
                            "nak wal record too large \
                             for buf seq={seq} len={}",
                            rec.payload.len()
                        );
                        continue;
                    }
                    self.buf[..WalHeader::SIZE]
                        .copy_from_slice(rec.header.to_bytes());
                    self.buf
                        [WalHeader::SIZE..total]
                        .copy_from_slice(&rec.payload);
                    if let Err(e) = self
                        .socket
                        .send_to(
                            &self.buf[..total], self.dest,
                        )
                    {
                        warn!(
                            "nak wal-retransmit send \
                             failed seq={seq}: {e}"
                        );
                    } else {
                        self.ring_last_retx_ns[slot] = now_ns;
                    }
                }
                Ok(None) => {
                    warn!(
                        "nak retransmit: seq={seq} \
                         not in ring or wal"
                    );
                }
                Err(e) => {
                    warn!(
                        "nak wal-read failed \
                         seq={seq}: {e}"
                    );
                }
            }
        }
    }

    pub fn recv_control(&mut self) {
        let mut cbuf = [0u8; 256];
        loop {
            match self.socket.recv_from(&mut cbuf) {
                Ok((n, _)) => {
                    if n < WalHeader::SIZE {
                        continue;
                    }
                    let hdr = match WalHeader::from_bytes(
                        &cbuf[..WalHeader::SIZE],
                    ) {
                        Some(h) => h,
                        None => continue,
                    };
                    let payload =
                        &cbuf[WalHeader::SIZE..n];
                    match hdr.record_type {
                        RECORD_NAK => {
                            if let Some(nak) =
                                decode_payload::<Nak>(payload)
                            {
                                self.handle_nak(&nak);
                            }
                        }
                        _ => {}
                    }
                }
                Err(ref e)
                    if e.kind()
                        == io::ErrorKind::WouldBlock =>
                {
                    break;
                }
                Err(_) => break,
            }
        }
    }

    /// Send raw bytes with explicit record_type.
    /// Does NOT assign seq (for non-CastRecord payloads).
    pub fn send_raw(
        &mut self,
        record_type: u16,
        payload: &[u8],
    ) -> io::Result<()> {
        frame_and_send(
            &self.socket,
            &mut self.buf,
            record_type,
            payload,
            self.dest,
        )?;
        self.last_heartbeat = Instant::now();
        Ok(())
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Advance seq after send_raw (for callers that
    /// set seq manually in the payload).
    pub fn advance_seq(&mut self) {
        self.next_seq += 1;
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

/// Receiver delivery outcome. Surfaces unrecoverable gaps
/// to the consumer explicitly rather than silently advancing.
///
/// Both `Faulted` and `Reconnect` are sticky until
/// `CastReceiver::reset_after_replay` is called. Both require
/// DXS/TCP replay — they differ only in cause.
#[derive(Debug)]
pub enum CastRecv {
    /// No data ready right now (would-block or queue empty).
    Empty,
    /// Next in-order record with its parsed header.
    Data(WalHeader, Vec<u8>),
    /// NAK retry budget exhausted: persistent gap. Consumer
    /// must switch to DXS/TCP replay from
    /// `last_delivered_seq + 1`, then call
    /// `reset_after_replay(new_tip)` to resume.
    Faulted {
        last_delivered_seq: u64,
        gap_start: u64,
        gap_end_inclusive: u64,
    },
    /// Reorder ring overflowed: gap > REORDER_CAPACITY (2048
    /// slots). Consumer must do a full DXS/TCP cold-start from
    /// `last_delivered_seq + 1`, then call
    /// `reset_after_replay(new_tip)` to resume.
    Reconnect {
        last_delivered_seq: u64,
    },
}

/// Zero-copy counterpart to [`CastRecv`], returned by
/// [`CastReceiver::poll`]. The payload is delivered via an
/// `FnOnce` callback that receives a `&[u8]` pointing directly
/// into the receiver's internal buffer — no heap allocation.
///
/// Use [`CastReceiver::try_recv`] if you need an owned `Vec<u8>`.
#[derive(Debug)]
pub enum CastRecvWith {
    Empty,
    Data,
    Faulted {
        last_delivered_seq: u64,
        gap_start: u64,
        gap_end_inclusive: u64,
    },
    Reconnect {
        last_delivered_seq: u64,
    },
}

pub struct CastReceiver {
    socket: UdpSocket,
    sender_addr: SocketAddr,
    /// Throttle: time of last "dropped" warning (e.g. on
    /// unsupported wire version). Avoids log-flood from a
    /// stuck or buggy peer.
    last_drop_warn: Instant,
    expected_seq: u64,
    highest_seen: u64,
    /// Sticky FAULTED state (NAK retry exhaustion).
    /// Cleared by reset_after_replay.
    faulted: bool,
    fault_last_delivered_seq: u64,
    fault_gap_start: u64,
    fault_gap_end_inclusive: u64,
    /// Sticky RECONNECT state (reorder ring overflow).
    /// Cleared by reset_after_replay.
    needs_reconnect: bool,
    reconnect_last_delivered_seq: u64,
    /// Per-gap NAK debounce ring. Indexed by
    /// `seq & REORDER_MASK`. Value = ns since `start_instant`
    /// of the last NAK sent for that `from_seq`. 0 = never
    /// NAKed. Zeroed on in-order delivery and on reset.
    nak_sent_at: Box<[u64]>,
    nak_retries_on_oldest: u16,
    /// Cached from config.
    nak_debounce_ns: u64,
    max_nak_retries: u16,
    /// Monotonic clock baseline for `nak_sent_at` values.
    start_instant: Instant,
    /// Out-of-order packet buffer. Three parallel arrays
    /// indexed by `seq & REORDER_MASK`. Empty slot iff
    /// `reorder_seqs[slot] == 0`. Slot conflict (wrapped
    /// ring) → `CastRecv::Reconnect`.
    reorder_seqs: Box<[u64]>,
    reorder_lens: Box<[u16]>,
    reorder_frames: Box<[u8]>,
    buf: [u8; PACKET_BUF_SIZE],
}

impl CastReceiver {
    pub fn new(
        bind_addr: SocketAddr,
        sender_addr: SocketAddr,
    ) -> io::Result<Self> {
        Self::with_config(
            bind_addr,
            sender_addr,
            &CastConfig::default(),
        )
    }

    pub fn with_config(
        bind_addr: SocketAddr,
        sender_addr: SocketAddr,
        config: &CastConfig,
    ) -> io::Result<Self> {
        let socket = bind_udp(bind_addr)?;
        Ok(Self {
            socket,
            sender_addr,
            last_drop_warn: Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
            expected_seq: 0,
            highest_seen: 0,
            faulted: false,
            fault_last_delivered_seq: 0,
            fault_gap_start: 0,
            fault_gap_end_inclusive: 0,
            needs_reconnect: false,
            reconnect_last_delivered_seq: 0,
            nak_sent_at: vec![0u64; REORDER_CAPACITY]
                .into_boxed_slice(),
            nak_retries_on_oldest: 0,
            nak_debounce_ns: config
                .nak_debounce_us
                .saturating_mul(1000),
            max_nak_retries: config.max_nak_retries,
            start_instant: Instant::now(),
            reorder_seqs: vec![0u64; REORDER_CAPACITY]
                .into_boxed_slice(),
            reorder_lens: vec![0u16; REORDER_CAPACITY]
                .into_boxed_slice(),
            reorder_frames: vec![
                0u8;
                REORDER_CAPACITY * REORDER_FRAME_BYTES
            ]
            .into_boxed_slice(),
            buf: [0u8; PACKET_BUF_SIZE],
        })
    }

    /// Clear FAULTED state and resume normal in-order
    /// delivery from `new_tip + 1`. Called by the consumer
    /// after DXS/TCP replay has caught up the application
    /// state to `new_tip`. Drops any stale reorder-buffered
    /// packets whose seqs are <= new_tip.
    ///
    /// `highest_seen` is **monotonic**. If `new_tip` is
    /// already below the current `highest_seen` (e.g. a
    /// heartbeat or out-of-order packet advanced
    /// `highest_seen` past the replay's stopping point),
    /// this method leaves `highest_seen` untouched. Lowering
    /// it would re-arm the gap detector against seqs the
    /// reorder ring may still hold and could cause the
    /// receiver to re-deliver records the consumer has
    /// already applied via replay. `expected_seq` always
    /// jumps to `new_tip + 1` regardless — that's the
    /// resume point for live-tail delivery.
    pub fn reset_after_replay(&mut self, new_tip: u64) {
        self.faulted = false;
        self.fault_last_delivered_seq = 0;
        self.fault_gap_start = 0;
        self.fault_gap_end_inclusive = 0;
        self.needs_reconnect = false;
        self.reconnect_last_delivered_seq = 0;
        self.nak_retries_on_oldest = 0;
        self.expected_seq = new_tip + 1;
        // Monotonic: only raise, never lower (see docstring).
        if self.highest_seen < new_tip + 1 {
            self.highest_seen = new_tip + 1;
        }
        // Clear stale reorder buffer and NAK debounce state.
        for s in self.reorder_seqs.iter_mut() {
            *s = 0;
        }
        for t in self.nak_sent_at.iter_mut() {
            *t = 0;
        }
        info!(
            "cmp receiver reset_after_replay: \
             expected_seq={}",
            self.expected_seq,
        );
    }

    #[cfg(test)]
    pub(crate) fn is_faulted(&self) -> bool {
        self.faulted
    }

    #[cfg(test)]
    pub(crate) fn is_reconnect_pending(&self) -> bool {
        self.needs_reconnect
    }

    /// Transition the receiver to FAULTED (NAK retry budget
    /// exhausted). Sticky until reset_after_replay().
    fn fault(
        &mut self,
        gap_start: u64,
        gap_end_inclusive: u64,
    ) {
        if self.faulted {
            return;
        }
        self.faulted = true;
        self.fault_last_delivered_seq =
            self.expected_seq.saturating_sub(1);
        self.fault_gap_start = gap_start;
        self.fault_gap_end_inclusive = gap_end_inclusive;
        warn!(
            "cmp receiver FAULTED: last_delivered={} \
             gap=[{}..={}]",
            self.fault_last_delivered_seq,
            gap_start,
            gap_end_inclusive,
        );
    }

    /// Per-gap NAK pump. Finds the oldest contiguous missing
    /// run and sends a NAK only if `nak_debounce_ns` has
    /// elapsed since the last NAK for that `from_seq`. After
    /// `max_nak_retries` debounce-windows without progress on
    /// the same gap, transitions to FAULTED.
    fn maybe_nak(&mut self, now_ns: u64) {
        if self.faulted || self.needs_reconnect {
            return;
        }
        let Some((from, count)) = self.oldest_missing_run()
        else {
            return;
        };
        // Per-gap debounce: only re-NAK if debounce window
        // has elapsed. First NAK for a new gap fires immediately
        // (slot value == 0).
        let slot = (from & REORDER_MASK) as usize;
        let last = self.nak_sent_at[slot];
        if last != 0
            && now_ns.saturating_sub(last)
                < self.nak_debounce_ns
        {
            return;
        }
        self.send_nak(from, count);
        self.nak_sent_at[slot] = now_ns;
        self.nak_retries_on_oldest =
            self.nak_retries_on_oldest.saturating_add(1);
        if self.nak_retries_on_oldest > self.max_nak_retries {
            self.fault(from, from + count - 1);
        }
    }

    /// Zero-copy receive. Calls `f` with the parsed header and
    /// a `&[u8]` pointing directly into the receiver's internal
    /// buffer — no heap allocation. Returns [`CastRecvWith::Data`]
    /// after invoking `f`, [`CastRecvWith::Empty`] when the socket
    /// would block with nothing ready in the reorder ring.
    ///
    /// `f` is dropped without being called on `Empty`, `Faulted`,
    /// and `Reconnect`. Both sticky states persist until
    /// `reset_after_replay` is called.
    pub fn try_recv_with<F>(&mut self, f: F) -> CastRecvWith
    where
        F: FnOnce(WalHeader, &[u8]),
    {
        if self.needs_reconnect {
            return CastRecvWith::Reconnect {
                last_delivered_seq: self
                    .reconnect_last_delivered_seq,
            };
        }
        if self.faulted {
            return CastRecvWith::Faulted {
                last_delivered_seq: self
                    .fault_last_delivered_seq,
                gap_start: self.fault_gap_start,
                gap_end_inclusive: self
                    .fault_gap_end_inclusive,
            };
        }
        // Option<F> lets the compiler verify f is called at
        // most once across the two delivery sites below.
        let mut f = Some(f);
        loop {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, _)) => {
                    if n < WalHeader::SIZE {
                        continue;
                    }
                    let hdr = match WalHeader::from_bytes(
                        &self.buf[..WalHeader::SIZE],
                    ) {
                        Some(h) => h,
                        None => {
                            let now = Instant::now();
                            if now.duration_since(
                                self.last_drop_warn,
                            ) >= Duration::from_secs(5)
                            {
                                warn!(
                                    "cmp: dropped datagram \
                                     with unrecognized \
                                     header (byte0={:#x})",
                                    self.buf[0],
                                );
                                self.last_drop_warn = now;
                            }
                            continue;
                        }
                    };
                    let payload_len = hdr.len as usize;
                    if payload_len > MAX_PAYLOAD {
                        continue;
                    }
                    if WalHeader::SIZE + payload_len > n {
                        continue;
                    }
                    let payload = &self.buf
                        [WalHeader::SIZE
                            ..WalHeader::SIZE
                                + payload_len];
                    let crc = compute_crc32(payload);
                    if crc != hdr.crc32 {
                        continue;
                    }

                    match hdr.record_type {
                        RECORD_HEARTBEAT => {
                            if let Some(hb) =
                                decode_payload::<CastHeartbeat>(
                                    payload,
                                )
                            {
                                if hb.highest_seq
                                    > self.highest_seen
                                {
                                    self.highest_seen =
                                        hb.highest_seq;
                                }
                                let now_ns = self
                                    .start_instant
                                    .elapsed()
                                    .as_nanos()
                                    as u64;
                                self.maybe_nak(now_ns);
                            }
                            continue;
                        }
                        RECORD_NAK => {
                            continue;
                        }
                        _ => {}
                    }

                    let seq = match extract_seq(payload) {
                        Some(s) => s,
                        None => continue,
                    };
                    if seq == 0 {
                        continue;
                    }

                    if self.expected_seq == 0 {
                        info!(
                            "cmp sync: first packet \
                             seq={}",
                            seq
                        );
                        self.expected_seq = seq;
                    }

                    if seq < self.expected_seq
                        && self.expected_seq - seq > 100
                    {
                        warn!(
                            "cmp sender reset detected: \
                             seq={} expected={}, re-sync",
                            seq, self.expected_seq
                        );
                        for s in self.reorder_seqs.iter_mut() {
                            *s = 0;
                        }
                        self.expected_seq = seq;
                    }

                    if seq < self.expected_seq {
                        continue;
                    }
                    if seq > self.highest_seen {
                        self.highest_seen = seq;
                    }

                    if seq == self.expected_seq {
                        self.expected_seq += 1;
                        self.nak_retries_on_oldest = 0;
                        let slot =
                            (seq & REORDER_MASK) as usize;
                        self.nak_sent_at[slot] = 0;
                        // Zero-copy: payload points into
                        // self.buf; f receives it directly.
                        f.take().unwrap()(hdr, payload);
                        return CastRecvWith::Data;
                    } else {
                        let total = WalHeader::SIZE
                            + payload.len();
                        let mut conflict = false;
                        if total <= REORDER_FRAME_BYTES {
                            let slot = (seq & REORDER_MASK)
                                as usize;
                            let existing =
                                self.reorder_seqs[slot];
                            if existing == 0
                                || existing == seq
                            {
                                self.reorder_seqs[slot] = seq;
                                self.reorder_lens[slot] =
                                    total as u16;
                                let off = slot
                                    * REORDER_FRAME_BYTES;
                                self.reorder_frames
                                    [off..off + total]
                                    .copy_from_slice(
                                        &self.buf[..total],
                                    );
                            } else {
                                conflict = true;
                            }
                        } else {
                            warn!(
                                "reorder: oversize record \
                                 dropped seq={} total={}",
                                seq, total,
                            );
                        }
                        if conflict {
                            if !self.needs_reconnect {
                                self.needs_reconnect = true;
                                self.reconnect_last_delivered_seq =
                                    self.expected_seq
                                        .saturating_sub(1);
                                warn!(
                                    "cmp reorder ring \
                                     overflow: \
                                     last_delivered={} \
                                     seq={} gap>{}",
                                    self.reconnect_last_delivered_seq,
                                    seq,
                                    REORDER_CAPACITY,
                                );
                            }
                            return CastRecvWith::Reconnect {
                                last_delivered_seq: self
                                    .reconnect_last_delivered_seq,
                            };
                        }
                        let now_ns = self
                            .start_instant
                            .elapsed()
                            .as_nanos()
                            as u64;
                        self.maybe_nak(now_ns);
                        if self.faulted {
                            return CastRecvWith::Faulted {
                                last_delivered_seq: self
                                    .fault_last_delivered_seq,
                                gap_start: self
                                    .fault_gap_start,
                                gap_end_inclusive: self
                                    .fault_gap_end_inclusive,
                            };
                        }
                        continue;
                    }
                }
                Err(ref e)
                    if e.kind()
                        == io::ErrorKind::WouldBlock =>
                {
                    break;
                }
                Err(_) => break,
            }
        }
        // Drain reorder ring: if the slot at expected_seq holds
        // a buffered packet, deliver it zero-copy via callback.
        if self.expected_seq != 0 {
            let slot =
                (self.expected_seq & REORDER_MASK) as usize;
            if self.reorder_seqs[slot] == self.expected_seq {
                let len =
                    self.reorder_lens[slot] as usize;
                let off = slot * REORDER_FRAME_BYTES;
                if let Some(hdr) = WalHeader::from_bytes(
                    &self.reorder_frames
                        [off..off + WalHeader::SIZE],
                ) {
                    {
                        let payload = &self.reorder_frames
                            [off + WalHeader::SIZE..off + len];
                        f.take().unwrap()(hdr, payload);
                    }
                    self.reorder_seqs[slot] = 0;
                    self.nak_sent_at[slot] = 0;
                    self.expected_seq += 1;
                    self.nak_retries_on_oldest = 0;
                    return CastRecvWith::Data;
                }
            }
        }
        CastRecvWith::Empty
    }

    /// Allocating shim over [`CastReceiver::poll`]. Allocates one
    /// `Vec<u8>` per in-order record. Prefer `poll` on the hot
    /// path.
    pub fn try_recv(&mut self) -> CastRecv {
        let mut out: Option<(WalHeader, Vec<u8>)> = None;
        match self.try_recv_with(|hdr, payload| {
            out = Some((hdr, payload.to_vec()));
        }) {
            CastRecvWith::Empty => CastRecv::Empty,
            CastRecvWith::Data => {
                let (hdr, payload) = out
                    // INVARIANT: poll sets out before Data.
                    .expect("poll Data invariant");
                CastRecv::Data(hdr, payload)
            }
            CastRecvWith::Faulted {
                last_delivered_seq,
                gap_start,
                gap_end_inclusive,
            } => CastRecv::Faulted {
                last_delivered_seq,
                gap_start,
                gap_end_inclusive,
            },
            CastRecvWith::Reconnect { last_delivered_seq } => {
                CastRecv::Reconnect { last_delivered_seq }
            }
        }
    }

    /// Locate the oldest contiguous run of missing seqs
    /// starting at `expected_seq`. Returns `(from_seq, count)`
    /// suitable for a NAK frame. `None` if the receiver is
    /// caught up.
    ///
    /// The run extends until we find a seq present in the
    /// ring, or until `highest_seen` — whichever is sooner.
    /// Worst case walks `REORDER_CAPACITY` slots; typical
    /// case (one missing seq) is one slot read.
    ///
    /// Clamps the upper bound to `expected_seq + REORDER_CAPACITY`
    /// to defend against a spoofed heartbeat with `highest_seq`
    /// near u64::MAX -- without the clamp, a single bad frame
    /// would walk ~2^64 slots and wedge the receiver thread.
    /// CMP trust model (specs/2/4-cast.md §10.4) delegates frame
    /// authentication to L3; it does NOT delegate "don't loop
    /// forever on attacker-controllable input." See CTO audit
    /// .ship/27 attack scenario A.
    pub(crate) fn oldest_missing_run(&self) -> Option<(u64, u64)> {
        if self.expected_seq == 0
            || self.expected_seq >= self.highest_seen + 1
        {
            return None;
        }
        let from = self.expected_seq;
        let upper = self
            .highest_seen
            .min(from.saturating_add(REORDER_CAPACITY as u64));
        let mut seq = from;
        while seq <= upper {
            let slot = (seq & REORDER_MASK) as usize;
            if self.reorder_seqs[slot] == seq {
                break;
            }
            seq += 1;
        }
        if seq == from {
            None
        } else {
            Some((from, seq - from))
        }
    }

    fn send_nak(&self, from_seq: u64, count: u64) {
        let nak = Nak {
            from_seq,
            count,
            _pad1: [0u8; 48],
        };
        let mut buf = [0u8; WalHeader::SIZE + 64];
        if let Err(e) = frame_and_send(
            &self.socket,
            &mut buf,
            RECORD_NAK,
            as_bytes(&nak),
            self.sender_addr,
        ) {
            warn!("cmp: send_nak failed: {e}");
        }
    }

    pub fn expected_seq(&self) -> u64 {
        self.expected_seq
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

#[cfg(test)]
#[path = "cast_test.rs"]
mod cast_test;
#[cfg(test)]
#[path = "cast_v4_test.rs"]
mod cast_v4_test;
#[cfg(test)]
#[path = "nak_fallback_latency_test.rs"]
mod nak_fallback_latency_test;
#[cfg(test)]
#[path = "sustained_throughput_test.rs"]
mod sustained_throughput_test;
