use crate::encode_utils::compute_crc32;
use crate::header::WalHeader;
use crate::records::CmpHeartbeat;
use crate::records::CmpRecord;
use crate::records::Nak;
use crate::records::RECORD_HEARTBEAT;
use crate::records::RECORD_NAK;
use crate::records::RECORD_STATUS_MESSAGE;
use crate::records::StatusMessage;
use crate::wal::WalReader;
use crate::wal::extract_seq;
use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::warn;

const REORDER_BUF_LIMIT: usize = 512;
const HEARTBEAT_INTERVAL: Duration =
    Duration::from_millis(10);
const STATUS_INTERVAL: Duration =
    Duration::from_millis(10);
const DEFAULT_WINDOW: u64 = 64 * 1024;

pub struct CmpSender {
    socket: UdpSocket,
    dest: SocketAddr,
    stream_id: u32,
    next_seq: u64,
    peer_consumption_seq: u64,
    peer_window: u64,
    last_heartbeat: Instant,
    wal_dir: PathBuf,
    buf: [u8; 65536],
}

impl CmpSender {
    pub fn new(
        dest: SocketAddr,
        stream_id: u32,
        wal_dir: &std::path::Path,
    ) -> io::Result<Self> {
        let socket =
            UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            dest,
            stream_id,
            next_seq: 1,
            peer_consumption_seq: 0,
            peer_window: DEFAULT_WINDOW,
            last_heartbeat: Instant::now(),
            wal_dir: wal_dir.to_path_buf(),
            buf: [0u8; 65536],
        })
    }

    /// Send a typed CMP record. Assigns seq via
    /// CmpRecord::set_seq. Returns false if flow
    /// control stalls.
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
        let crc = compute_crc32(payload);
        let header = WalHeader::new(
            T::record_type(),
            payload.len() as u16,
            crc,
        );
        let hdr_bytes = header.to_bytes();
        let total = WalHeader::SIZE + payload.len();
        self.buf[..WalHeader::SIZE]
            .copy_from_slice(&hdr_bytes);
        self.buf[WalHeader::SIZE..total]
            .copy_from_slice(payload);
        self.socket
            .send_to(&self.buf[..total], self.dest)?;
        self.next_seq += 1;
        Ok(true)
    }

    pub fn tick(&mut self) -> io::Result<()> {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat)
            >= HEARTBEAT_INTERVAL
        {
            let hb = CmpHeartbeat {
                highest_seq: self.next_seq
                    .saturating_sub(1),
                _pad1: [0u8; 56],
            };
            let payload = as_bytes(&hb);
            let crc = compute_crc32(payload);
            let header = WalHeader::new(
                RECORD_HEARTBEAT,
                payload.len() as u16,
                crc,
            );
            let hdr_bytes = header.to_bytes();
            let total =
                WalHeader::SIZE + payload.len();
            self.buf[..WalHeader::SIZE]
                .copy_from_slice(&hdr_bytes);
            self.buf[WalHeader::SIZE..total]
                .copy_from_slice(payload);
            let _ = self.socket.send_to(
                &self.buf[..total],
                self.dest,
            );
            self.last_heartbeat = now;
        }
        Ok(())
    }

    pub fn handle_status(&mut self, msg: &StatusMessage) {
        self.peer_consumption_seq = msg.consumption_seq;
        self.peer_window = msg.receiver_window;
    }

    pub fn handle_nak(&mut self, nak: &Nak) {
        let mut reader = match WalReader::open_from_seq(
            self.stream_id,
            nak.from_seq,
            &self.wal_dir,
        ) {
            Ok(r) => r,
            Err(e) => {
                warn!("nak retransmit open: {e}");
                return;
            }
        };
        for _ in 0..nak.count {
            match reader.next() {
                Ok(Some(record)) => {
                    let hdr_bytes =
                        record.header.to_bytes();
                    let total = WalHeader::SIZE
                        + record.payload.len();
                    self.buf[..WalHeader::SIZE]
                        .copy_from_slice(&hdr_bytes);
                    self.buf[WalHeader::SIZE..total]
                        .copy_from_slice(
                            &record.payload,
                        );
                    let _ = self.socket.send_to(
                        &self.buf[..total],
                        self.dest,
                    );
                }
                _ => break,
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
        let crc = compute_crc32(payload);
        let header = WalHeader::new(
            record_type,
            payload.len() as u16,
            crc,
        );
        let hdr_bytes = header.to_bytes();
        let total = WalHeader::SIZE + payload.len();
        self.buf[..WalHeader::SIZE]
            .copy_from_slice(&hdr_bytes);
        self.buf[WalHeader::SIZE..total]
            .copy_from_slice(payload);
        self.socket
            .send_to(&self.buf[..total], self.dest)?;
        Ok(true)
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
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
    expected_seq: u64,
    highest_seen: u64,
    reorder_buf: BTreeMap<u64, Vec<u8>>,
    last_status: Instant,
    window: u64,
    buf: [u8; 65536],
}

impl CmpReceiver {
    pub fn new(
        bind_addr: SocketAddr,
        sender_addr: SocketAddr,
        _stream_id: u32,
    ) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind_addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            sender_addr,
            expected_seq: 1,
            highest_seen: 0,
            reorder_buf: BTreeMap::new(),
            last_status: Instant::now(),
            window: DEFAULT_WINDOW,
            buf: [0u8; 65536],
        })
    }

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
                    let payload_len = hdr.len as usize;
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
                                if self.highest_seen
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
                    } else {
                        if self.reorder_buf.len()
                            < REORDER_BUF_LIMIT
                        {
                            let mut full =
                                Vec::with_capacity(
                                    WalHeader::SIZE
                                        + payload_len,
                                );
                            full.extend_from_slice(
                                &self.buf
                                    [..WalHeader::SIZE],
                            );
                            full.extend_from_slice(
                                payload,
                            );
                            self.reorder_buf
                                .insert(seq, full);
                        }
                        self.send_nak(
                            self.expected_seq,
                            seq - self.expected_seq,
                        );
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
        let payload = as_bytes(&nak);
        let crc = compute_crc32(payload);
        let header = WalHeader::new(
            RECORD_NAK,
            payload.len() as u16,
            crc,
        );
        let hdr_bytes = header.to_bytes();
        let mut buf =
            [0u8; WalHeader::SIZE + 64];
        buf[..WalHeader::SIZE]
            .copy_from_slice(&hdr_bytes);
        buf[WalHeader::SIZE
            ..WalHeader::SIZE + payload.len()]
            .copy_from_slice(payload);
        let total =
            WalHeader::SIZE + payload.len();
        let _ = self.socket.send_to(
            &buf[..total],
            self.sender_addr,
        );
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_status)
            >= STATUS_INTERVAL
        {
            let msg = StatusMessage {
                consumption_seq: self
                    .expected_seq
                    .saturating_sub(1),
                receiver_window: self.window,
                _pad1: [0u8; 48],
            };
            let payload = as_bytes(&msg);
            let crc = compute_crc32(payload);
            let header = WalHeader::new(
                RECORD_STATUS_MESSAGE,
                payload.len() as u16,
                crc,
            );
            let hdr_bytes = header.to_bytes();
            let mut buf =
                [0u8; WalHeader::SIZE + 64];
            buf[..WalHeader::SIZE]
                .copy_from_slice(&hdr_bytes);
            buf[WalHeader::SIZE
                ..WalHeader::SIZE + payload.len()]
                .copy_from_slice(payload);
            let total =
                WalHeader::SIZE + payload.len();
            let _ = self.socket.send_to(
                &buf[..total],
                self.sender_addr,
            );
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

fn as_bytes<T>(val: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            val as *const T as *const u8,
            std::mem::size_of::<T>(),
        )
    }
}
