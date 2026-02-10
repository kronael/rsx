use crate::protocol::serialize_l2_snapshot;
use crate::shadow::ShadowBook;
use crate::subscription::SubscriptionManager;
use crate::types::BboUpdate;
use rsx_types::SymbolConfig;
use std::collections::HashMap;
use std::collections::VecDeque;

pub struct ConnectionState {
    pub outbound: VecDeque<String>,
}

pub struct MarketDataState {
    next_conn_id: u64,
    connections: HashMap<u64, ConnectionState>,
    subs: SubscriptionManager,
    books: Vec<Option<ShadowBook>>,
    last_bbo: Vec<Option<BboUpdate>>,
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
    ) {
        if let Some(conn) = self.connections.get_mut(&conn_id)
        {
            if conn.outbound.len() >= max_outbound {
                return;
            }
            conn.outbound.push_back(msg);
        }
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

    pub fn snapshot_msg(
        &self,
        symbol_id: u32,
        depth: u32,
    ) -> Option<String> {
        let book = self.books.get(symbol_id as usize)?;
        let book = book.as_ref()?;
        Some(serialize_l2_snapshot(&book.derive_l2_snapshot(
            depth as usize,
        )))
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
}
