use crate::protocol::serialize_l2_snapshot;
use crate::shadow::ShadowBook;
use crate::subscription::SubscriptionManager;
use crate::types::BboUpdate;
use rsx_types::SymbolConfig;
use rsx_types::time::time_ns;
use std::collections::HashMap;
use std::collections::VecDeque;

pub struct ConnectionState {
    pub outbound: VecDeque<String>,
    pub last_heartbeat_ns: u64,
}

pub struct MarketDataState {
    next_conn_id: u64,
    connections: HashMap<u64, ConnectionState>,
    subs: SubscriptionManager,
    books: Vec<Option<ShadowBook>>,
    last_bbo: Vec<Option<BboUpdate>>,
    expected_seq: Vec<u64>,
    gap_count: u64,
    base_config: SymbolConfig,
    book_capacity: u32,
    mid_price_default: i64,
}

impl MarketDataState {
    pub fn new(
        max_symbols: usize,
        base_config: SymbolConfig,
        book_capacity: u32,
        mid_price_default: i64,
    ) -> Self {
        Self {
            next_conn_id: 0,
            connections: HashMap::new(),
            subs: SubscriptionManager::new(),
            books: (0..max_symbols).map(|_| None).collect(),
            last_bbo: (0..max_symbols).map(|_| None).collect(),
            expected_seq: vec![0; max_symbols],
            gap_count: 0,
            base_config,
            book_capacity,
            mid_price_default,
        }
    }

    pub fn add_connection(&mut self) -> u64 {
        let id = self.next_conn_id;
        self.next_conn_id += 1;
        self.connections.insert(
            id,
            ConnectionState {
                outbound: VecDeque::new(),
                last_heartbeat_ns: time_ns(),
            },
        );
        id
    }

    pub fn remove_connection(&mut self, conn_id: u64) {
        self.connections.remove(&conn_id);
        self.subs.unsubscribe_all(conn_id);
    }

    pub fn push_to_client(
        &mut self,
        conn_id: u64,
        msg: String,
        max_outbound: usize,
    ) -> bool {
        if let Some(conn) = self.connections.get_mut(&conn_id)
        {
            if conn.outbound.len() >= max_outbound {
                return false;
            }
            conn.outbound.push_back(msg);
            return true;
        }
        false
    }

    pub fn drain_outbound(
        &mut self,
        conn_id: u64,
    ) -> Vec<String> {
        if let Some(conn) = self.connections.get_mut(&conn_id)
        {
            conn.outbound.drain(..).collect()
        } else {
            Vec::new()
        }
    }

    pub fn subscribe(
        &mut self,
        conn_id: u64,
        symbol_id: u32,
        channels: u32,
        depth: u32,
    ) -> bool {
        self.subs.subscribe(conn_id, symbol_id, channels, depth)
    }

    pub fn unsubscribe(
        &mut self,
        conn_id: u64,
        symbol_id: u32,
    ) {
        self.subs.unsubscribe(conn_id, symbol_id);
    }

    pub fn unsubscribe_all(&mut self, conn_id: u64) {
        self.subs.unsubscribe_all(conn_id);
    }

    pub fn clients_for_symbol(&self, symbol_id: u32) -> Vec<u64> {
        self.subs.clients_for_symbol(symbol_id)
    }

    pub fn has_bbo(&self, conn_id: u64, symbol_id: u32) -> bool {
        self.subs.has_bbo(conn_id, symbol_id)
    }

    pub fn has_depth(
        &self,
        conn_id: u64,
        symbol_id: u32,
    ) -> bool {
        self.subs.has_depth(conn_id, symbol_id)
    }

    pub fn has_trades(
        &self,
        conn_id: u64,
        symbol_id: u32,
    ) -> bool {
        self.subs.has_trades(conn_id, symbol_id)
    }

    pub fn snapshot_msg(
        &self,
        symbol_id: u32,
        depth: u32,
    ) -> Option<String> {
        let book = self.books.get(symbol_id as usize)?;
        if let Some(book) = book.as_ref() {
            return Some(serialize_l2_snapshot(
                &book.derive_l2_snapshot(depth as usize),
            ));
        }
        // empty-book snapshot
        Some(serialize_l2_snapshot(
            &crate::types::L2Snapshot {
                symbol_id,
                bids: Vec::new(),
                asks: Vec::new(),
                timestamp_ns: 0,
                seq: 0,
            },
        ))
    }

    pub fn client_depth(&self, conn_id: u64) -> u32 {
        self.subs.client_depth(conn_id)
    }

    pub fn ensure_book(&mut self, symbol_id: u32, mid_price_hint: i64) {
        let idx = symbol_id as usize;
        if idx >= self.books.len() {
            return;
        }
        if self.books[idx].is_some() {
            return;
        }
        let mut cfg = self.base_config.clone();
        cfg.symbol_id = symbol_id;
        let mid = if mid_price_hint > 0 {
            mid_price_hint
        } else {
            self.mid_price_default
        };
        self.books[idx] = Some(ShadowBook::new(
            cfg,
            self.book_capacity,
            mid,
        ));
    }

    pub fn book_mut(&mut self, symbol_id: u32) -> Option<&mut ShadowBook> {
        self.books
            .get_mut(symbol_id as usize)?
            .as_mut()
    }

    pub fn last_bbo_mut(&mut self, symbol_id: u32) -> Option<&mut Option<BboUpdate>> {
        self.last_bbo.get_mut(symbol_id as usize)
    }

    /// Track sequence for a symbol. Returns true if a gap
    /// was detected (caller should trigger snapshot resend).
    pub fn check_seq(
        &mut self,
        symbol_id: u32,
        seq: u64,
    ) -> bool {
        let idx = symbol_id as usize;
        if idx >= self.expected_seq.len() || seq == 0 {
            return false;
        }
        let expected = self.expected_seq[idx];
        if expected == 0 {
            // first seq seen for this symbol
            self.expected_seq[idx] = seq + 1;
            return false;
        }
        if seq == expected {
            self.expected_seq[idx] = seq + 1;
            return false;
        }
        if seq > expected {
            // gap detected
            self.gap_count += 1;
            self.expected_seq[idx] = seq + 1;
            return true;
        }
        // seq < expected: duplicate, ignore
        false
    }

    pub fn gap_count(&self) -> u64 {
        self.gap_count
    }

    /// Broadcast L2 snapshot to all depth subscribers
    /// for a symbol (used after seq gap detection).
    pub fn resend_snapshot(
        &mut self,
        symbol_id: u32,
        depth: u32,
        max_outbound: usize,
    ) {
        if let Some(snapshot) = self.snapshot_msg(symbol_id, depth)
        {
            let clients = self.subs.clients_for_symbol(symbol_id);
            for client_id in clients {
                if self.subs.has_depth(client_id, symbol_id) {
                    let _ = self.push_to_client(
                        client_id,
                        snapshot.clone(),
                        max_outbound,
                    );
                }
            }
        }
    }

    pub fn send_snapshot_to_client(
        &mut self,
        client_id: u64,
        symbol_id: u32,
        depth: u32,
        max_outbound: usize,
    ) {
        if let Some(conn) = self.connections.get_mut(&client_id)
        {
            conn.outbound.clear();
        }
        if let Some(snapshot) = self.snapshot_msg(symbol_id, depth)
        {
            let _ = self.push_to_client(
                client_id,
                snapshot,
                max_outbound,
            );
        }
    }

    pub fn broadcast_heartbeat(&mut self, ts_ms: u64) {
        let msg = format!("{{\"H\":[{}]}}", ts_ms);
        for conn in self.connections.values_mut() {
            conn.outbound.push_back(msg.clone());
        }
    }

    pub fn update_heartbeat(&mut self, conn_id: u64) {
        if let Some(conn) = self.connections.get_mut(&conn_id) {
            conn.last_heartbeat_ns = time_ns();
        }
    }

    pub fn check_timeouts(&mut self, timeout_ns: u64) -> Vec<u64> {
        let now = time_ns();
        let mut timed_out = Vec::new();
        for (conn_id, conn) in &self.connections {
            if now - conn.last_heartbeat_ns >= timeout_ns {
                timed_out.push(*conn_id);
            }
        }
        for conn_id in &timed_out {
            self.remove_connection(*conn_id);
        }
        timed_out
    }
}
