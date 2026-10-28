use rsx_cast::cast::CastSender;
use rsx_cast::wal::oldest_and_highest_seq;
use rsx_cast::wal::Framed;
use rsx_cast::wal::WalWriter;
use rsx_cast::CastRecord;
use rsx_messages::CancelRequest;
use rsx_messages::OrderRequest;
use rsx_messages::GW_STREAM_ID;
use rtrb::Producer;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;
use tracing::warn;

/// Response to gateway after pre-trade check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum OrderResponse {
    Accepted {
        user_id: u32,
        margin_reserved: i64,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    Rejected {
        user_id: u32,
        reason: crate::types::RejectReason,
        order_id_hi: u64,
        order_id_lo: u64,
    },
    /// Notice to a liquidated user: `status` 0 is a placed round,
    /// 3 a socialized loss.
    Liquidation {
        user_id: u32,
        symbol_id: u32,
        status: u8,
        round: u32,
        side: u8,
        qty: i64,
        price: i64,
        slip_bps: i64,
    },
}

/// A command for the ME that owns the symbol. `Order` carries the
/// accepted order and its risk ingress timestamp.
#[derive(Clone, Copy, Debug)]
pub enum MeCommand {
    Order(OrderRequest, u64),
    Cancel(CancelRequest),
}

/// Egress SPSC ring endpoints for one risk shard. The recv loop
/// calls `process_*` directly (same thread); these two carry the
/// shard's output back to the live loop's casting senders.
pub struct ShardRings {
    pub response_producer: Producer<OrderResponse>,
    pub me_producer: Producer<MeCommand>,
}

const GW_WAL_FILE_BYTES: u64 = 64 * 1024 * 1024;
const GW_WAL_FLUSH_INTERVAL: Duration = Duration::from_millis(10);

/// Risk's gateway egress stream: forwarded ME records and risk-generated
/// rejects and notices, numbered by this stream's own contiguous seq (a
/// forwarded record keeps its order id, not the ME's seq). Each record is
/// framed once, appended to the stream's WAL and cast, so a NAK
/// retransmit and the gateway's replay are served from disk. The WAL
/// flushes on `tick`, so a record is on disk within `GW_WAL_FLUSH_INTERVAL`
/// of being cast.
pub struct GwStream {
    writer: WalWriter,
    sender: CastSender,
    last_flush: Instant,
}

impl GwStream {
    /// Open the stream's WAL under `wal_dir`, numbering new records after
    /// the highest seq already on disk, and a sender to `dest`.
    pub fn open(dest: SocketAddr, wal_dir: &Path) -> io::Result<Self> {
        let mut writer = WalWriter::new(GW_STREAM_ID, wal_dir, GW_WAL_FILE_BYTES)?;
        if let Some((_, highest)) = oldest_and_highest_seq(GW_STREAM_ID, wal_dir)? {
            writer.set_next_seq(highest + 1);
        }
        let sender = CastSender::new(dest, GW_STREAM_ID, wal_dir)?;
        Ok(Self {
            writer,
            sender,
            last_flush: Instant::now(),
        })
    }

    /// The sender's bound address: where the receiver sends NAKs.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sender.local_addr()
    }

    /// Stamp `record` with the stream's next seq, append it to the WAL
    /// buffer and cast it. A full WAL buffer stalls until a flush frees
    /// it; a failed cast only warns, since the WAL holds the record.
    pub fn forward<T: CastRecord>(&mut self, record: &mut T) {
        let framed = match self.writer.prepare(record) {
            Ok(framed) => framed,
            Err(error) => {
                warn!("risk: gw frame failed: {error}");
                return;
            }
        };
        self.append(&framed);
        if let Err(error) = self.sender.send_framed(&framed) {
            warn!("risk: forward to gw failed: {error}");
        }
    }

    /// Flush the WAL once `GW_WAL_FLUSH_INTERVAL` has passed since the
    /// last flush or the buffer holds enough records, then send a
    /// heartbeat if the stream is idle and serve NAKs. `now` is the
    /// loop's cached clock.
    pub fn tick(&mut self, now: Instant) {
        let due = now.duration_since(self.last_flush) >= GW_WAL_FLUSH_INTERVAL;
        if due || self.writer.should_flush() {
            if let Err(error) = self.writer.flush() {
                warn!("risk: gw wal flush failed, retrying next tick: {error}");
            }
            self.last_flush = now;
        }
        if let Err(error) = self.sender.tick() {
            warn!("risk: gw_sender tick failed: {error}");
        }
        self.sender.recv_control();
    }

    /// Flush the WAL buffer to disk.
    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    fn append(&mut self, framed: &Framed) {
        let mut stalled = false;
        loop {
            match self.writer.append_framed(framed) {
                Ok(()) => return,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !stalled {
                        warn!("risk: gw wal buffer full, stalling until flushed");
                        stalled = true;
                    }
                    if let Err(error) = self.writer.flush() {
                        warn!("risk: gw wal flush failed while stalled: {error}");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                Err(error) => {
                    warn!("risk: gw wal append failed: {error}");
                    return;
                }
            }
        }
    }
}
