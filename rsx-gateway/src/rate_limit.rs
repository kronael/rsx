use std::time::Instant;

pub struct RateLimiter {
    tokens: f64,
    capacity: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(capacity: u32, refill_rate: u32) -> Self {
        Self {
            tokens: capacity as f64,
            capacity: capacity as f64,
            refill_rate: refill_rate as f64,
            last_refill: Instant::now(),
        }
    }

    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now
            .duration_since(self.last_refill)
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate)
            .min(self.capacity);
        self.last_refill = now;
    }

    pub fn tokens_remaining(&self) -> f64 {
        self.tokens
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
