use std::time::Instant;

const MICROS_PER_SEC: i64 = 1_000_000;

pub struct RateLimiter {
    tokens: i64,
    capacity: i64,
    refill_rate: i64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(capacity: u32, refill_rate: u32) -> Self {
        let cap_micros = capacity as i64 * MICROS_PER_SEC;
        Self {
            tokens: cap_micros,
            capacity: cap_micros,
            refill_rate: refill_rate as i64,
            last_refill: Instant::now(),
        }
    }

    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= MICROS_PER_SEC {
            self.tokens -= MICROS_PER_SEC;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed_micros = now
            .duration_since(self.last_refill)
            .as_micros() as i64;
        let refill_amount = elapsed_micros
            .saturating_mul(self.refill_rate);
        self.tokens =
            self.tokens.saturating_add(refill_amount)
                .min(self.capacity);
        self.last_refill = now;
    }

    pub fn tokens_remaining(&self) -> f64 {
        self.tokens as f64 / MICROS_PER_SEC as f64
    }

    pub fn advance_time_by(
        &mut self,
        duration: std::time::Duration,
    ) {
        let now = self.last_refill + duration;
        let elapsed_micros = duration.as_micros() as i64;
        let refill_amount = elapsed_micros
            .saturating_mul(self.refill_rate);
        self.tokens =
            self.tokens.saturating_add(refill_amount)
                .min(self.capacity);
        self.last_refill = now;
    }
}

pub fn per_user() -> RateLimiter {
    RateLimiter::new(10, 10)
}

pub fn per_ip() -> RateLimiter {
    RateLimiter::new(100, 100)
}

pub fn per_instance() -> RateLimiter {
    RateLimiter::new(1000, 1000)
}
