use crate::circuit::CircuitBreaker;
use crate::client_ids::ClientOrderIds;
use crate::egress::EgressWaker;
use crate::metrics::GatewayMetrics;
use crate::pending::PendingOrders;
use crate::rate_limit::RateLimiter;
use crate::symbols::SymbolTable;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::net::IpAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tracing::info;
use tracing::warn;

/// Hard cap on the per-IP limiter map. Above this, we evict
/// the oldest entry on each new insert (FIFO). Bounds memory
/// against an adversary rotating source IPs, while still
/// catching legitimate misbehaviour from any single IP for
/// long enough to matter.
pub const IP_LIMITER_MAX: usize = 10_000;

/// Default cap on a connection's queued outbound messages.
pub const DEFAULT_MAX_OUTBOUND: usize = 1024;

/// Default orders per second the gateway instance admits in total.
pub const DEFAULT_RATE_LIMIT_PER_INSTANCE: u32 = 1000;

/// How long `Closing` waits for sessions to send their close frame and end.
const CLOSE_GRACE_NS: u64 = 1_000_000_000;

/// Where the gateway is on its way out. `Draining` refuses new orders until
/// no `Submitted` order is left or the drain bound passes; `Closing` has told
/// every session to send close 1001 and end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Serving,
    Draining,
    Closing,
}

/// Per-connection state.
pub struct ConnectionState {
    pub user_id: u32,
    pub outbound: VecDeque<Arc<str>>,
    /// Woken by producers right after they queue into `outbound`,
    /// so the handler drains + writes without polling on a timer.
    pub egress: Rc<EgressWaker>,
    pub last_activity_ns: u64,
}

/// Shared gateway state (single-threaded, Rc<RefCell>).
pub struct GatewayState {
    pub connections: FxHashMap<u64, ConnectionState>,
    pub pending: PendingOrders,
    /// Order ids bound to client order ids for the dedup window.
    pub client_ids: ClientOrderIds,
    pub next_conn_id: u64,
    pub user_limiters: FxHashMap<u32, RateLimiter>,
    pub ip_limiters: FxHashMap<IpAddr, RateLimiter>,
    /// Insertion order for ip_limiters, used for FIFO eviction
    /// once the map reaches IP_LIMITER_MAX entries.
    pub ip_limiter_order: VecDeque<IpAddr>,
    /// Orders per second across every user and address.
    pub instance_limiter: RateLimiter,
    pub circuit: CircuitBreaker,
    pub symbols: SymbolTable,
    /// Rate limit capacities (per-user, per-ip). Used when
    /// the corresponding limiter is first created.
    pub rate_limit_per_user: u32,
    pub rate_limit_per_ip: u32,
    /// Cap on each connection's `outbound`. A connection that would
    /// exceed it is closed: order updates and fills cannot be dropped
    /// or coalesced, so the client reconnects instead.
    pub max_outbound: usize,
    /// Counters behind `/metrics`; the reactor thread is their only writer.
    pub metrics: Arc<GatewayMetrics>,
    /// When the state was created; `/health` reports the age as `uptime_sec`.
    pub started_at: Instant,
    pub phase: Phase,
    phase_ends_ns: u64,
}

impl GatewayState {
    pub fn new(
        max_pending: usize,
        circuit_threshold: u32,
        circuit_cooldown_ms: u64,
        symbols: SymbolTable,
    ) -> Self {
        Self {
            connections: FxHashMap::default(),
            pending: PendingOrders::new(max_pending),
            client_ids: ClientOrderIds::default(),
            next_conn_id: 0,
            user_limiters: FxHashMap::default(),
            ip_limiters: FxHashMap::default(),
            ip_limiter_order: VecDeque::with_capacity(IP_LIMITER_MAX),
            instance_limiter: RateLimiter::new(
                DEFAULT_RATE_LIMIT_PER_INSTANCE,
                DEFAULT_RATE_LIMIT_PER_INSTANCE,
            ),
            circuit: CircuitBreaker::new(
                circuit_threshold,
                Duration::from_millis(circuit_cooldown_ms),
            ),
            symbols,
            rate_limit_per_user: 10,
            rate_limit_per_ip: 100,
            max_outbound: DEFAULT_MAX_OUTBOUND,
            metrics: Arc::new(GatewayMetrics::default()),
            started_at: Instant::now(),
            phase: Phase::Serving,
            phase_ends_ns: 0,
        }
    }

    /// Take the next step of a shutdown at `now_ns` and return true once the
    /// gateway may exit. The first call starts `Draining` with a bound of
    /// `drain_ns`; `Draining` ends when no `Submitted` order is left or the
    /// bound passes; `Closing` ends when every session is gone or after
    /// `CLOSE_GRACE_NS`.
    pub fn advance_shutdown(&mut self, now_ns: u64, drain_ns: u64) -> bool {
        match self.phase {
            Phase::Serving => {
                self.phase = Phase::Draining;
                self.phase_ends_ns = now_ns + drain_ns;
                self.metrics.draining.set(1);
                false
            }
            Phase::Draining => {
                if self.pending.in_flight() == 0 || now_ns >= self.phase_ends_ns {
                    self.phase = Phase::Closing;
                    self.phase_ends_ns = now_ns + CLOSE_GRACE_NS;
                    for conn in self.connections.values() {
                        conn.egress.signal();
                    }
                }
                false
            }
            Phase::Closing => self.connections.is_empty() || now_ns >= self.phase_ends_ns,
        }
    }

    /// Admit at most `per_sec` orders per second across the instance.
    pub fn set_instance_rate_limit(&mut self, per_sec: u32) {
        self.instance_limiter = RateLimiter::new(per_sec, per_sec);
    }

    /// Whether a `CONFIG_APPLIED` for `symbol_id` at `config_version` names a
    /// version the table does not hold yet, so the table is worth re-reading.
    pub fn is_config_newer(&self, symbol_id: u32, config_version: u64) -> bool {
        self.symbols
            .get(&symbol_id)
            .is_none_or(|entry| entry.config_version < config_version)
    }

    pub fn add_connection(&mut self, user_id: u32) -> Result<u64, &'static str> {
        let count = self
            .connections
            .values()
            .filter(|c| c.user_id == user_id)
            .count();
        if count >= 5 {
            return Err("max connections per user");
        }
        let id = self.next_conn_id;
        self.next_conn_id += 1;
        self.connections.insert(
            id,
            ConnectionState {
                user_id,
                outbound: VecDeque::new(),
                egress: Rc::new(EgressWaker::default()),
                last_activity_ns: 0,
            },
        );
        Ok(id)
    }

    /// Drop the connection's state and close its egress, which ends
    /// the handler session and its socket.
    pub fn remove_connection(&mut self, conn_id: u64) {
        if let Some(conn) = self.connections.remove(&conn_id) {
            conn.egress.close();
        }
    }

    pub fn push_to_user(&mut self, user_id: u32, msg: Arc<str>) {
        self.queue_where(&msg, |_, conn| conn.user_id == user_id);
    }

    /// Queue `{H:[ts_ms]}` for every connection.
    pub fn broadcast_heartbeat(&mut self, ts_ms: u64) {
        self.queue_where(&heartbeat_message(ts_ms), |_, _| true);
    }

    /// Queue `{H:[ts_ms]}` for one connection: the greeting a new
    /// session opens with.
    pub fn queue_heartbeat(&mut self, conn_id: u64, ts_ms: u64) {
        self.queue_where(&heartbeat_message(ts_ms), |id, _| id == conn_id);
    }

    /// Queue `msg` on every connection `wanted` accepts and wake its
    /// handler. A connection whose queue is full is closed.
    fn queue_where(&mut self, msg: &Arc<str>, wanted: impl Fn(u64, &ConnectionState) -> bool) {
        let mut overflowed = Vec::new();
        for (conn_id, conn) in self.connections.iter_mut() {
            if wanted(*conn_id, conn) && !queue_outbound(conn, msg, self.max_outbound) {
                overflowed.push(*conn_id);
            }
        }
        for conn_id in overflowed {
            warn!(
                "closing connection {} whose outbound queue reached {}",
                conn_id, self.max_outbound
            );
            self.remove_connection(conn_id);
        }
    }

    /// Remove every connection whose last inbound frame is older than
    /// `cutoff_ns`; each handler closes its socket. Returns the count.
    pub fn reap_idle_connections(&mut self, cutoff_ns: u64) -> usize {
        let idle: Vec<u64> = self
            .connections
            .iter()
            .filter(|(_, conn)| conn.last_activity_ns > 0 && conn.last_activity_ns < cutoff_ns)
            .map(|(conn_id, _)| *conn_id)
            .collect();
        for conn_id in &idle {
            if let Some(conn) = self.connections.get(conn_id) {
                info!("closing idle connection user_id={}", conn.user_id);
            }
            self.remove_connection(*conn_id);
        }
        idle.len()
    }

    pub fn touch_connection(&mut self, conn_id: u64, now_ns: u64) {
        if let Some(conn) = self.connections.get_mut(&conn_id) {
            conn.last_activity_ns = now_ns;
        }
    }

    pub fn drain_outbound(&mut self, conn_id: u64) -> Vec<Arc<str>> {
        if let Some(conn) = self.connections.get_mut(&conn_id) {
            conn.outbound.drain(..).collect()
        } else {
            Vec::new()
        }
    }

    /// Handle to a connection's egress wake, so its handler task can
    /// park on it. Cloned once at connection setup; the producers
    /// signal the original via the connection map.
    pub fn connection_egress(&self, conn_id: u64) -> Option<Rc<EgressWaker>> {
        self.connections.get(&conn_id).map(|c| c.egress.clone())
    }

    /// Get-or-insert a per-IP rate limiter. Bounded by IP_LIMITER_MAX; a
    /// new IP at capacity evicts the oldest (FIFO).
    pub fn ip_limiter_for(&mut self, ip: IpAddr) -> &mut RateLimiter {
        if !self.ip_limiters.contains_key(&ip) {
            if self.ip_limiters.len() >= IP_LIMITER_MAX {
                if let Some(oldest) = self.ip_limiter_order.pop_front() {
                    self.ip_limiters.remove(&oldest);
                }
            }
            self.ip_limiter_order.push_back(ip);
        }
        let cap = self.rate_limit_per_ip;
        self.ip_limiters
            .entry(ip)
            .or_insert_with(|| RateLimiter::new(cap, cap))
    }
}

fn heartbeat_message(ts_ms: u64) -> Arc<str> {
    format!("{{\"H\":[{}]}}", ts_ms).into()
}

/// Queue `msg` for the connection and wake its handler. False when the
/// queue already holds `max_outbound` messages; `msg` is then not queued.
fn queue_outbound(conn: &mut ConnectionState, msg: &Arc<str>, max_outbound: usize) -> bool {
    if conn.outbound.len() >= max_outbound {
        return false;
    }
    conn.outbound.push_back(msg.clone());
    conn.egress.signal();
    true
}
