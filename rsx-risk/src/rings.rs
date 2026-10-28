use rsx_cast::cast::CastSender;
use rsx_cast::config::CastConfig;
use rsx_cast::wal::oldest_and_highest_seq;
use rsx_cast::wal::Framed;
use rsx_cast::wal::WalWriter;
use rsx_cast::CastRecord;
use rsx_messages::CancelRequest;
use rsx_messages::OrderRequest;
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

const EGRESS_WAL_FILE_BYTES: u64 = 64 * 1024 * 1024;
const EGRESS_WAL_FLUSH_INTERVAL: Duration = Duration::from_millis(10);

/// One WAL-backed cast stream risk writes: the gateway's (forwarded ME
/// records, rejects and notices) or one ME's (orders and cancels). Each is
/// numbered by its own contiguous seq: a forwarded record keeps its order
/// id, not the ME's seq, and the gateway's seq would read as a gap at an
/// ME. A record is framed once, appended to the stream's WAL and cast, so a
/// NAK retransmit and a replay are served from disk. The WAL flushes on
/// `tick`, so a record is on disk within `EGRESS_WAL_FLUSH_INTERVAL` of
/// being cast.
pub struct EgressStream {
    stream_id: u32,
    writer: WalWriter,
    sender: CastSender,
    last_flush: Instant,
}

impl EgressStream {
    /// Bind a sender to `dest` as `config` says, then open the stream's WAL
    /// under `wal_dir`, numbering new records after the highest seq already
    /// on disk. A node that cannot bind the sender never opens the WAL.
    /// Streams that share a `wal_dir` need distinct `stream_id`s.
    pub fn open(
        stream_id: u32,
        dest: SocketAddr,
        wal_dir: &Path,
        config: &CastConfig,
    ) -> io::Result<Self> {
        let sender = CastSender::with_config(dest, stream_id, wal_dir, config)?;
        let mut writer = WalWriter::new(stream_id, wal_dir, EGRESS_WAL_FILE_BYTES)?;
        if let Some((_, highest)) = oldest_and_highest_seq(stream_id, wal_dir)? {
            writer.set_next_seq(highest + 1);
        }
        Ok(Self {
            stream_id,
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
    /// it. `Err` only when the record reached neither the WAL nor the
    /// wire; a failure of one alone warns, since the other holds the record
    /// and the receiver NAKs it off the next heartbeat.
    pub fn forward<T: CastRecord>(&mut self, record: &mut T) -> io::Result<()> {
        let framed = self.writer.prepare(record)?;
        let appended = self.append(&framed);
        let sent = self.sender.send_framed(&framed);
        match (appended, sent) {
            (Err(error), Err(_)) => Err(error),
            (Err(error), Ok(())) => {
                warn!("risk: stream {} wal append failed: {error}", self.stream_id);
                Ok(())
            }
            (Ok(()), Err(error)) => {
                warn!("risk: stream {} cast failed: {error}", self.stream_id);
                Ok(())
            }
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    /// Flush the WAL once `EGRESS_WAL_FLUSH_INTERVAL` has passed since the
    /// last flush or the buffer holds enough records, then send a
    /// heartbeat if the stream is idle and serve NAKs. `now` is the
    /// loop's cached clock.
    pub fn tick(&mut self, now: Instant) {
        let due = now.duration_since(self.last_flush) >= EGRESS_WAL_FLUSH_INTERVAL;
        if due || self.writer.should_flush() {
            if let Err(error) = self.writer.flush() {
                warn!(
                    "risk: stream {} wal flush failed, retrying next tick: {error}",
                    self.stream_id
                );
            }
            self.last_flush = now;
        }
        if let Err(error) = self.sender.tick() {
            warn!("risk: stream {} tick failed: {error}", self.stream_id);
        }
        self.sender.recv_control();
    }

    /// Flush the WAL buffer to disk.
    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    fn append(&mut self, framed: &Framed) -> io::Result<()> {
        let mut stalled = false;
        loop {
            match self.writer.append_framed(framed) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if !stalled {
                        warn!(
                            "risk: stream {} wal buffer full, stalling until flushed",
                            self.stream_id
                        );
                        stalled = true;
                    }
                    if let Err(error) = self.writer.flush() {
                        warn!(
                            "risk: stream {} wal flush failed while stalled: {error}",
                            self.stream_id
                        );
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                done => return done,
            }
        }
    }
}
