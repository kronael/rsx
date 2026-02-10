use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

pub struct CircuitBreaker {
    state: State,
    failure_count: u32,
    threshold: u32,
    cooldown: Duration,
    last_failure: Option<Instant>,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            state: State::Closed,
            failure_count: 0,
            threshold,
            cooldown,
            last_failure: None,
        }
    }

    pub fn allow(&mut self) -> bool {
        match self.state {
            State::Closed => true,
            State::Open => {
                if let Some(last) = self.last_failure {
                    if last.elapsed() >= self.cooldown {
                        self.state = State::HalfOpen;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            State::HalfOpen => false,
        }
    }

    pub fn record_success(&mut self) {
        match self.state {
            State::HalfOpen => {
                self.state = State::Closed;
                self.failure_count = 0;
            }
            State::Closed => {
                self.failure_count = 0;
            }
            _ => {}
        }
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());
        match self.state {
            State::Closed => {
                if self.failure_count >= self.threshold {
                    self.state = State::Open;
                }
            }
            State::HalfOpen => {
                self.state = State::Open;
            }
            _ => {}
        }
    }

    pub fn state(&self) -> State {
        self.state
    }
}
