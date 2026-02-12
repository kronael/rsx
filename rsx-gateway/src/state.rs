use crate::circuit::CircuitBreaker;
use crate::pending::PendingOrders;
use crate::rate_limit::RateLimiter;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::net::IpAddr;
use std::time::Duration;
use rsx_types::SymbolConfig;

/// Per-connection state.
pub struct ConnectionState {
    pub user_id: u32,
    pub outbound: VecDeque<String>,
    pub last_activity_ns: u64,
}

/// Shared gateway state (single-threaded, Rc<RefCell>).
pub struct GatewayState {
    pub connections: FxHashMap<u64, ConnectionState>,
    pub pending: PendingOrders,
    pub next_conn_id: u64,
    pub user_limiters: FxHashMap<u32, RateLimiter>,
    pub ip_limiters: FxHashMap<IpAddr, RateLimiter>,
    pub circuit: CircuitBreaker,
    pub symbol_configs: Vec<SymbolConfig>,
    pub config_versions: Vec<u64>,
}

impl GatewayState {
    pub fn new(
        max_pending: usize,
        circuit_threshold: u32,
        circuit_cooldown_ms: u64,
        symbol_configs: Vec<SymbolConfig>,
    ) -> Self {
        Self {
            connections: FxHashMap::default(),
            pending: PendingOrders::new(max_pending),
            next_conn_id: 0,
            user_limiters: FxHashMap::default(),
            ip_limiters: FxHashMap::default(),
            circuit: CircuitBreaker::new(
                circuit_threshold,
                Duration::from_millis(circuit_cooldown_ms),
            ),
            config_versions: vec![0; symbol_configs.len()],
            symbol_configs,
        }
    }

    pub fn apply_config_applied(
        &mut self,
        symbol_id: u32,
        config_version: u64,
    ) -> bool {
        let sid = symbol_id as usize;
        if sid >= self.config_versions.len() {
            return false;
        }
        if config_version < self.config_versions[sid] {
            return false;
        }
        self.config_versions[sid] = config_version;
        self.reload_symbol_overrides(symbol_id);
        true
    }

    fn reload_symbol_overrides(&mut self, symbol_id: u32) {
        let sid = symbol_id as usize;
        if sid >= self.symbol_configs.len() {
            return;
        }
        let tick_key =
            format!("RSX_SYMBOL_{}_TICK_SIZE", symbol_id);
        if let Ok(v) = std::env::var(&tick_key) {
            if let Ok(parsed) = v.parse::<i64>() {
                self.symbol_configs[sid].tick_size = parsed;
            }
        }
        let lot_key =
            format!("RSX_SYMBOL_{}_LOT_SIZE", symbol_id);
        if let Ok(v) = std::env::var(&lot_key) {
            if let Ok(parsed) = v.parse::<i64>() {
                self.symbol_configs[sid].lot_size = parsed;
            }
        }
        let pd_key = format!(
            "RSX_SYMBOL_{}_PRICE_DECIMALS",
            symbol_id
        );
        if let Ok(v) = std::env::var(&pd_key) {
            if let Ok(parsed) = v.parse::<u8>() {
                self.symbol_configs[sid].price_decimals = parsed;
            }
        }
        let qd_key =
            format!("RSX_SYMBOL_{}_QTY_DECIMALS", symbol_id);
        if let Ok(v) = std::env::var(&qd_key) {
            if let Ok(parsed) = v.parse::<u8>() {
                self.symbol_configs[sid].qty_decimals = parsed;
            }
        }
    }

    pub fn add_connection(
        &mut self,
        user_id: u32,
    ) -> Result<u64, &'static str> {
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
                last_activity_ns: 0,
            },
        );
        Ok(id)
    }

    pub fn remove_connection(&mut self, conn_id: u64) {
        self.connections.remove(&conn_id);
    }

    pub fn push_to_user(
        &mut self,
        user_id: u32,
        msg: String,
    ) {
        for conn in self.connections.values_mut() {
            if conn.user_id == user_id {
                conn.outbound.push_back(msg.clone());
            }
        }
    }

    pub fn broadcast_heartbeat(
        &mut self,
        ts_ms: u64,
    ) {
        let msg =
            format!("{{\"H\":[{}]}}", ts_ms);
        for conn in self.connections.values_mut() {
            conn.outbound.push_back(msg.clone());
        }
    }

    pub fn stale_connections(
        &self,
        cutoff_ns: u64,
    ) -> Vec<u64> {
        self.connections
            .iter()
            .filter(|(_, c)| {
                c.last_activity_ns > 0
                    && c.last_activity_ns < cutoff_ns
            })
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn touch_connection(
        &mut self,
        conn_id: u64,
        now_ns: u64,
    ) {
        if let Some(conn) =
            self.connections.get_mut(&conn_id)
        {
            conn.last_activity_ns = now_ns;
        }
    }

    pub fn drain_outbound(
        &mut self,
        conn_id: u64,
    ) -> Vec<String> {
        if let Some(conn) =
            self.connections.get_mut(&conn_id)
        {
            conn.outbound.drain(..).collect()
        } else {
            Vec::new()
        }
    }
}
