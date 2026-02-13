use crate::types::NewOrder;
use rand::Rng;

#[derive(Clone)]
pub struct SymbolConfig {
    pub symbol_id: u32,
    pub name: String,
    pub mid_price: i64,
    pub tick_size: i64,
    pub lot_size: i64,
    pub weight: f64,
}

pub struct OrderGenerator {
    symbols: Vec<SymbolConfig>,
    users: Vec<u32>,
    user_idx: usize,
    counter: u64,
    rng: rand::rngs::StdRng,
}

impl OrderGenerator {
    pub fn new(symbols: Vec<SymbolConfig>, users: Vec<u32>) -> Self {
        use rand::SeedableRng;
        Self {
            symbols,
            users,
            user_idx: 0,
            counter: 0,
            rng: rand::rngs::StdRng::from_entropy(),
        }
    }

    pub fn next_user(&mut self) -> u32 {
        let user = self.users[self.user_idx];
        self.user_idx = (self.user_idx + 1) % self.users.len();
        user
    }

    pub fn next_order(&mut self) -> NewOrder {
        self.counter += 1;

        let r: f64 = self.rng.gen();
        let mut cumulative = 0.0;
        let mut selected = &self.symbols[0];
        for sym in &self.symbols {
            cumulative += sym.weight;
            if r < cumulative {
                selected = sym;
                break;
            }
        }

        let side: u8 = if self.rng.gen_bool(0.5) { 0 } else { 1 };

        let price_offset = self.rng.gen_range(-100..=100);
        let price = selected.mid_price + (price_offset * selected.tick_size);

        let qty_multiplier = self.rng.gen_range(1..=100);
        let qty = qty_multiplier * selected.lot_size;

        let timestamp_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let client_order_id = format!("{:016x}{:04x}", timestamp_ns, self.counter & 0xffff);
        let client_order_id = format!("{:0<20}", client_order_id);

        NewOrder {
            symbol_id: selected.symbol_id,
            side,
            price,
            qty,
            client_order_id,
            tif: 0,
            reduce_only: false,
            post_only: false,
        }
    }
}

impl Default for OrderGenerator {
    fn default() -> Self {
        let symbols = vec![
            SymbolConfig {
                symbol_id: 1,
                name: "BTCUSD".to_string(),
                mid_price: 50000_00,
                tick_size: 1_00,
                lot_size: 1_00,
                weight: 0.5,
            },
            SymbolConfig {
                symbol_id: 2,
                name: "ETHUSD".to_string(),
                mid_price: 3000_00,
                tick_size: 1_00,
                lot_size: 1_00,
                weight: 0.3,
            },
            SymbolConfig {
                symbol_id: 3,
                name: "SOLUSD".to_string(),
                mid_price: 100_00,
                tick_size: 1_00,
                lot_size: 1_00,
                weight: 0.2,
            },
        ];
        let users = vec![1001, 1002, 1003, 1004, 1005];
        Self::new(symbols, users)
    }
}
