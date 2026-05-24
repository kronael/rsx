use crate::config::CmpConfig;
use crate::encode_utils::as_bytes;
use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::CmpHeartbeat;
use crate::records::CmpRecord;
use crate::records::Nak;
use crate::records::RECORD_HEARTBEAT;
use crate::records::RECORD_NAK;
use crate::records::RECORD_STATUS_MESSAGE;
use crate::records::StatusMessage;
use crate::wal::extract_seq;
use crate::wal::read_record_at_seq;
use socket2::Domain;
use socket2::Protocol;
use socket2::Socket;
use socket2::Type;
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

/// Bind a UDP socket with SO_REUSEADDR (+ SO_REUSEPORT on
/// Linux) so a restarted process can claim the same port
/// even while the dead parent's socket lingers in
/// TIME_WAIT/half-open. Prevents the CMP restart loop that
/// otherwise violates spec invariant 7 (WAL persistence):
/// each AddrInUse panic truncates the active WAL file.
fn bind_udp_reuse(addr: SocketAddr) -> io::Result<UdpSocket> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(target_os = "linux")]
    socket.set_reuse_port(true)?;
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
    let hdr = WalHeader::new(
        record_type,
        payload.len() as u16,
        crc,
    )
    .to_bytes();
    let total = WalHeader::SIZE + payload.len();
    buf[..WalHeader::SIZE].copy_from_slice(&hdr);
    buf[WalHeader::SIZE..total].copy_from_slice(payload);
    socket.send_to(&buf[..total], dest)?;
    Ok(total)
}

pub struct CmpSender {
    socket: UdpSocket,
    dest: SocketAddr,
    next_seq: u64,
    peer_consumption_seq: u64,
    peer_window: u64,
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
    /// allocations on the hot send path** (the prior
    /// `BTreeMap<u64, Vec<u8>>` heap-allocated per send and
    /// per cleanup).
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

impl CmpSender {
    pub fn new(
        dest: SocketAddr,
        stream_id: u32,
        wal_dir: &std::path::Path,
    ) -> io::Result<Self> {
        Self::with_config(
            dest,
            stream_id,
            wal_dir,
            &CmpConfig::default(),
        )
    }

    pub fn with_config(
        dest: SocketAddr,
        stream_id: u32,
        wal_dir: &std::path::Path,
        config: &CmpConfig,
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
        let socket = bind_udp_reuse(bind)?;
        Ok(Self {
            socket,
            dest,
            next_seq: 1,
            peer_consumption_seq: 0,
            peer_window: config.default_window,
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
    /// CmpRecord::set_seq. Returns false if flow
    /// control stalls.
    ///
    /// Hot send path: zero heap allocations. The send-ring
    /// retransmit cache, `buf`, and `ring_frames` are
    /// preallocated; the only call sites that touch the
    /// allocator are construction and NAK fallback (cold
    /// path via `read_record_at_seq`). NB: the *receive*
    /// path (`CmpReceiver::try_recv`) does heap-allocate
    /// one `Vec<u8>` per in-order packet — the zero-heap
    /// claim is for the send path only.
    pub fn send<T: CmpRecord>(
        &mut self,
        record: &mut T,
    ) -> io::Result<bool> {
        let seq = self.next_seq;
        let limit = self.peer_consumption_seq
            + self.peer_window;
        if seq > limit && limit > 0 {
            return Ok(false);
        }

        record.set_seq(seq);

        let payload = as_bytes(record);
        let total = frame_and_send(
            &self.socket,
            &mut self.buf,
            T::record_type(),
            payload,
            self.dest,
        )?;

        // Cache the frame in the preallocated ring for
        // NAK retransmit. Skip frames larger than the
        // slot — those are recoverable via WAL fallback.
        if total <= SEND_RING_FRAME_BYTES {
            let slot =
                (seq & SEND_RING_MASK) as usize;
            self.ring_seqs[slot] = seq;
            self.ring_lens[slot] = total as u16;
            let off = slot * SEND_RING_FRAME_BYTES;
            self.ring_frames[off..off + total]
                .copy_from_slice(&self.buf[..total]);
        } else {
            // Mark slot dirty: if a NAK targets this seq,
            // the seq mismatch fall-through pushes it to
            // WAL, which is correct.
            let slot =
                (seq & SEND_RING_MASK) as usize;
            self.ring_seqs[slot] = 0;
            self.ring_lens[slot] = 0;
        }

        self.next_seq += 1;
        Ok(true)
    }

    pub fn tick(&mut self) -> io::Result<()> {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat)
            >= self.heartbeat_interval
        {
            let hb = CmpHeartbeat {
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

    pub fn handle_status(&mut self, msg: &StatusMessage) {
        self.peer_consumption_seq = msg.consumption_seq;
        self.peer_window = msg.receiver_window;
    }

    pub fn handle_nak(&mut self, nak: &Nak) {
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
            // cadence is bounded by `nak_retry_us` (100 µs
            // default) so legitimate retries fall through.
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
                None,
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
                    let hdr = rec.header.to_bytes();
                    self.buf[..WalHeader::SIZE]
                        .copy_from_slice(&hdr);
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
                    if !hdr.is_supported_version() {
                        // Sender's control reply uses
                        // unknown wire version; drop.
                        continue;
                    }
                    let payload =
                        &cbuf[WalHeader::SIZE..n];
                    match hdr.record_type {
                        RECORD_STATUS_MESSAGE => {
                            if payload.len()
                                >= std::mem::size_of::<
                                    StatusMessage,
                                >()
                            {
                                let msg = unsafe {
                                    std::ptr::read_unaligned(
                                        payload.as_ptr()
                                            as *const StatusMessage,
                                    )
                                };
                                self.handle_status(&msg);
                            }
                        }
                        RECORD_NAK => {
                            if payload.len()
                                >= std::mem::size_of::<
                                    Nak,
                                >()
                            {
                                let nak = unsafe {
                                    std::ptr::read_unaligned(
                                        payload.as_ptr()
                                            as *const Nak,
                                    )
                                };
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
    /// Does NOT assign seq (for non-CmpRecord payloads).
    pub fn send_raw(
        &mut self,
        record_type: u16,
        payload: &[u8],
    ) -> io::Result<bool> {
        frame_and_send(
            &self.socket,
            &mut self.buf,
            record_type,
            payload,
            self.dest,
        )?;
        Ok(true)
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Advance seq after send_raw (for callers that
    /// set seq manually in the payload).
    pub fn advance_seq(&mut self) {
        self.next_seq += 1;
    }

    pub fn peer_consumption_seq(&self) -> u64 {
        self.peer_consumption_seq
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

pub struct CmpReceiver {
    socket: UdpSocket,
    sender_addr: SocketAddr,
    /// Throttle: time of last "dropped" warning (e.g. on
    /// unsupported wire version). Avoids log-flood from a
    /// stuck or buggy peer.
    last_drop_warn: Instant,
    expected_seq: u64,
    highest_seen: u64,
    /// Out-of-order packet buffer. Heap-allocates per inserted
    /// packet. Acceptable because: (1) bounded at
    /// `reorder_buf_limit` (default 512) — overflow drops the
    /// oldest gap and re-syncs; (2) on a trusted LAN the
    /// happy path is in-order delivery, so this allocator
    /// rarely fires; (3) NAK retransmits go through the
    /// preallocated `send_ring` on the sender, not here.
    /// Keeping the simpler BTreeMap avoids a second slab when
    /// the path it guards is cold.
    reorder_buf: BTreeMap<u64, Vec<u8>>,
    reorder_buf_limit: usize,
    last_status: Instant,
    status_interval: Duration,
    window: u64,
    buf: [u8; PACKET_BUF_SIZE],
}

impl CmpReceiver {
    pub fn new(
        bind_addr: SocketAddr,
        sender_addr: SocketAddr,
        _stream_id: u32,
    ) -> io::Result<Self> {
        Self::with_config(
            bind_addr,
            sender_addr,
            _stream_id,
            &CmpConfig::default(),
        )
    }

    pub fn with_config(
        bind_addr: SocketAddr,
        sender_addr: SocketAddr,
        _stream_id: u32,
        config: &CmpConfig,
    ) -> io::Result<Self> {
        let socket = bind_udp_reuse(bind_addr)?;
        Ok(Self {
            socket,
            sender_addr,
            last_drop_warn: Instant::now()
                .checked_sub(Duration::from_secs(60))
                .unwrap_or_else(Instant::now),
            expected_seq: 0,
            highest_seen: 0,
            reorder_buf: BTreeMap::new(),
            reorder_buf_limit: config
                .reorder_buf_limit,
            last_status: Instant::now(),
            status_interval: Duration::from_millis(
                config.status_interval_ms,
            ),
            window: config.default_window,
            buf: [0u8; PACKET_BUF_SIZE],
        })
    }

    /// Receive the next in-order CMP packet, if any. Returns
    /// the parsed header plus an owned copy of the payload.
    ///
    /// NB: this allocates a `Vec<u8>` for the payload on every
    /// in-order delivery — the receive path is NOT zero-heap.
    /// The zero-heap claim in the project docs applies to
    /// `CmpSender::send` only. A zero-copy receive variant
    /// (caller-supplied `&mut [u8]`) is tracked as future work;
    /// since all current consumers (risk, marketdata) do
    /// further work proportional to the payload, the per-packet
    /// alloc is not on the measured GW→ME→GW critical path.
    pub fn try_recv(
        &mut self,
    ) -> Option<(WalHeader, Vec<u8>)> {
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
                        None => continue,
                    };
                    if !hdr.is_supported_version() {
                        // Unknown wire version; receiver
                        // doesn't know how to interpret the
                        // payload, drop. Throttled to avoid
                        // log-flood under attack or bug.
                        let now = Instant::now();
                        if now
                            .duration_since(self.last_drop_warn)
                            >= Duration::from_secs(5)
                        {
                            warn!(
                                "cmp: dropped datagram with \
                                 unsupported version v{}",
                                hdr.version
                            );
                            self.last_drop_warn = now;
                        }
                        continue;
                    }
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
                            if payload_len
                                >= std::mem::size_of::<
                                    CmpHeartbeat,
                                >()
                            {
                                let hb = unsafe {
                                    std::ptr::read_unaligned(
                                        payload.as_ptr()
                                            as *const CmpHeartbeat,
                                    )
                                };
                                if hb.highest_seq
                                    > self.highest_seen
                                {
                                    self.highest_seen =
                                        hb.highest_seq;
                                }
                                if self.expected_seq > 0
                                    && self.highest_seen
                                        > self.expected_seq
                                {
                                    self.send_nak(
                                        self.expected_seq,
                                        self.highest_seen
                                            - self
                                                .expected_seq,
                                    );
                                }
                            }
                            continue;
                        }
                        RECORD_STATUS_MESSAGE
                        | RECORD_NAK => {
                            continue;
                        }
                        _ => {}
                    }

                    // Extract seq from payload
                    // (CmpRecord convention: first 8 bytes)
                    let seq = match extract_seq(payload) {
                        Some(s) => s,
                        None => continue,
                    };
                    if seq == 0 {
                        continue;
                    }

                    // First packet: sync to sender's seq
                    if self.expected_seq == 0 {
                        info!(
                            "cmp sync: first packet \
                             seq={}",
                            seq
                        );
                        self.expected_seq = seq;
                    }

                    // Sender restart: seq dropped below
                    // expected by large margin — re-sync
                    if seq < self.expected_seq
                        && self.expected_seq - seq > 100
                    {
                        warn!(
                            "cmp sender reset detected: \
                             seq={} expected={}, re-sync",
                            seq, self.expected_seq
                        );
                        self.reorder_buf.clear();
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
                        let data =
                            payload.to_vec();
                        self.drain_reorder();
                        return Some((hdr, data));
                    } else if self.reorder_buf.len()
                        < self.reorder_buf_limit
                    {
                        let full = [
                            &self.buf[..WalHeader::SIZE],
                            payload,
                        ]
                        .concat();
                        self.reorder_buf.insert(seq, full);
                        self.send_nak(
                            self.expected_seq,
                            seq - self.expected_seq,
                        );
                        continue;
                    } else {
                        warn!(
                            "reorder buf full ({}), \
                             skip gap {}..{}",
                            self.reorder_buf_limit,
                            self.expected_seq,
                            seq,
                        );
                        self.reorder_buf.clear();
                        self.expected_seq = seq + 1;
                        return Some((
                            hdr,
                            payload.to_vec(),
                        ));
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
        self.try_drain_reorder()
    }

    fn drain_reorder(&mut self) {
        while let Some(entry) =
            self.reorder_buf.first_entry()
        {
            if *entry.key() == self.expected_seq {
                entry.remove();
                self.expected_seq += 1;
            } else {
                break;
            }
        }
    }

    fn try_drain_reorder(
        &mut self,
    ) -> Option<(WalHeader, Vec<u8>)> {
        if let Some(entry) =
            self.reorder_buf.first_entry()
        {
            if *entry.key() == self.expected_seq {
                let data = entry.remove();
                self.expected_seq += 1;
                self.drain_reorder();
                let hdr = WalHeader::from_bytes(
                    &data[..WalHeader::SIZE],
                )?;
                let payload =
                    data[WalHeader::SIZE..].to_vec();
                return Some((hdr, payload));
            }
        }
        None
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

    pub fn tick(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_status)
            >= self.status_interval
        {
            let msg = StatusMessage {
                consumption_seq: self
                    .expected_seq
                    .saturating_sub(1),
                receiver_window: self.window,
                _pad1: [0u8; 48],
            };
            let mut buf = [0u8; WalHeader::SIZE + 64];
            if let Err(e) = frame_and_send(
                &self.socket,
                &mut buf,
                RECORD_STATUS_MESSAGE,
                as_bytes(&msg),
                self.sender_addr,
            ) {
                warn!("cmp: status send failed: {e}");
            }
            self.last_status = now;
        }
    }

    pub fn expected_seq(&self) -> u64 {
        self.expected_seq
    }

    pub fn highest_seen(&self) -> u64 {
        self.highest_seen
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}
