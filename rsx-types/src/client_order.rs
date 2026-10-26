//! Client-side order construction: client order ids and minimum-size
//! quantities for in-repo order generators (probes, benches).
//!
//! The gateway binds a client order id to its order per user for an hour and
//! answers a repeat as a duplicate (specs/2/61-cid-idempotency.md), and risk
//! rejects an order whose `price * qty` is below the symbol's minimum
//! notional (`ORDER_TOO_SMALL`). A generator that repeats a cid, or sends
//! less than the minimum, measures the reject path instead of the match path.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::hash::Hasher;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

/// Width of the random session part of a cid. 36^8 is about 2^41, above the
/// 40 random bits spec 61 asks of a client.
pub const SESSION_CHARS: usize = 8;

/// Base36 digits of `number`, lowercase, no leading zeros.
pub fn base36(mut number: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut reversed = Vec::new();
    loop {
        reversed.push(DIGITS[(number % 36) as usize]);
        number /= 36;
        if number == 0 {
            break;
        }
    }
    reversed.reverse();
    String::from_utf8(reversed).unwrap_or_default()
}

/// Client order ids of one generator process: a random session part drawn at
/// start and a base36 counter. 1-20 ASCII chars, never padded.
pub struct CidMint {
    session: String,
    counter: u64,
}

impl CidMint {
    /// A mint with a session unique to this process.
    pub fn new() -> Self {
        Self::with_seed(process_entropy())
    }

    /// A mint whose session is derived from `seed`.
    pub fn with_seed(seed: u64) -> Self {
        let span = 36_u64.pow(SESSION_CHARS as u32);
        let digits = base36(seed % span);
        Self {
            session: format!("{digits:0>width$}", width = SESSION_CHARS),
            counter: 0,
        }
    }

    /// The fixed-width session part every cid of this mint starts with.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// The next cid: session part, then the counter in base36 from 1.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> String {
        self.counter += 1;
        format!("{}{}", self.session, base36(self.counter))
    }
}

impl Default for CidMint {
    fn default() -> Self {
        Self::new()
    }
}

/// 64 bits that differ per process: the per-process random hasher keys mixed
/// with the clock and the process id.
fn process_entropy() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0),
    );
    hasher.write_u32(std::process::id());
    hasher.finish()
}

/// 1 USD in raw `price * qty` units: `10^(price_decimals + qty_decimals)`, the
/// minimum notional the preset symbols carry. Saturates at `i64::MAX`.
pub fn one_usd_notional(price_decimals: u32, qty_decimals: u32) -> i64 {
    10_i64
        .checked_pow(price_decimals + qty_decimals)
        .unwrap_or(i64::MAX)
}

/// Smallest quantity, a multiple of `lot`, with `price * qty` at or above
/// `min_notional` (risk's check, computed in i128). At least one lot;
/// saturates at the largest lot multiple an i64 holds.
pub fn min_order_qty(min_notional: i64, price: i64, lot: i64) -> i64 {
    let lot = lot.max(1);
    let per_lot = i128::from(price.max(1)) * i128::from(lot);
    let lots = (i128::from(min_notional.max(0)) + per_lot - 1) / per_lot;
    let lots = lots.clamp(1, i128::from(i64::MAX / lot));
    (lots as i64) * lot
}

#[cfg(test)]
#[path = "client_order_test.rs"]
mod client_order_test;
