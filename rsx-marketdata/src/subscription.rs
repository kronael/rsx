use std::collections::HashMap;
use std::collections::HashSet;

pub const CHANNEL_BBO: u32 = 1;
pub const CHANNEL_DEPTH: u32 = 2;
pub const CHANNEL_TRADES: u32 = 4;

#[derive(Debug, Clone)]
pub struct ClientSubscription {
    pub symbols: HashMap<u32, u32>,
    pub depth: u32,
}

pub struct SubscriptionManager {
    clients: HashMap<u64, ClientSubscription>,
    symbol_clients: HashMap<u32, HashSet<u64>>,
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            symbol_clients: HashMap::new(),
        }
    }

    /// Subscribe a client to a symbol with given channels.
    /// Returns true if this is a new subscription.
    pub fn subscribe(
        &mut self,
        client_id: u64,
        symbol_id: u32,
        channels: u32,
        depth: u32,
    ) -> bool {
        let entry = self.clients
            .entry(client_id)
            .or_insert_with(|| ClientSubscription {
                symbols: HashMap::new(),
                depth,
            });
        let is_new = !entry.symbols.contains_key(&symbol_id);
        entry.symbols.insert(symbol_id, channels);
        entry.depth = depth;
        self.symbol_clients
            .entry(symbol_id)
            .or_default()
            .insert(client_id);
        is_new
    }

    /// Unsubscribe a client from a symbol.
    pub fn unsubscribe(
        &mut self,
        client_id: u64,
        symbol_id: u32,
    ) {
        if let Some(sub) = self.clients.get_mut(&client_id) {
            sub.symbols.remove(&symbol_id);
        }
        if let Some(set) =
            self.symbol_clients.get_mut(&symbol_id)
        {
            set.remove(&client_id);
        }
    }

    /// Unsubscribe a client from all symbols.
    pub fn unsubscribe_all(&mut self, client_id: u64) {
        if let Some(sub) = self.clients.remove(&client_id) {
            for symbol_id in sub.symbols.keys() {
                if let Some(set) =
                    self.symbol_clients.get_mut(symbol_id)
                {
                    set.remove(&client_id);
                }
            }
        }
    }

    /// Get all client IDs subscribed to a symbol.
    pub fn clients_for_symbol(
        &self,
        symbol_id: u32,
    ) -> Vec<u64> {
        self.symbol_clients
            .get(&symbol_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Check if client is subscribed to BBO for a symbol.
    pub fn has_bbo(
        &self,
        client_id: u64,
        symbol_id: u32,
    ) -> bool {
        self.clients
            .get(&client_id)
            .and_then(|s| s.symbols.get(&symbol_id))
            .map(|ch| ch & CHANNEL_BBO != 0)
            .unwrap_or(false)
    }

    /// Check if client is subscribed to depth for a symbol.
    pub fn has_depth(
        &self,
        client_id: u64,
        symbol_id: u32,
    ) -> bool {
        self.clients
            .get(&client_id)
            .and_then(|s| s.symbols.get(&symbol_id))
            .map(|ch| ch & CHANNEL_DEPTH != 0)
            .unwrap_or(false)
    }

    /// Check if client is subscribed to trades for a symbol.
    pub fn has_trades(
        &self,
        client_id: u64,
        symbol_id: u32,
    ) -> bool {
        self.clients
            .get(&client_id)
            .and_then(|s| s.symbols.get(&symbol_id))
            .map(|ch| ch & CHANNEL_TRADES != 0)
            .unwrap_or(false)
    }

    /// Get the depth parameter for a client.
    pub fn client_depth(
        &self,
        client_id: u64,
    ) -> u32 {
        self.clients
            .get(&client_id)
            .map(|s| s.depth)
            .unwrap_or(10)
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}
