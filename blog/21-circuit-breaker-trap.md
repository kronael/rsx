# The HalfOpen Deadlock

A circuit breaker has three states. Most implementations get two of
them right.

## The Pattern

Closed: normal operation, requests flow through.
Open: failures exceeded threshold, all requests rejected.
HalfOpen: cooldown elapsed, one probe request allowed through.

The probe succeeds → back to Closed. The probe fails → back to Open.

HalfOpen is the recovery state. It exists for one reason: to let the
system test whether the downstream service has recovered without
immediately exposing it to full traffic.

## The Bug

The naive implementation of HalfOpen looks like this:

```rust
State::HalfOpen => false,
```

Fail-safe. Reject everything while broken. Intuitive. Wrong.

We had a version of this. The code returned `false` for every request
in HalfOpen. No probe could get through. The circuit was permanently
stuck — never closed, never open, rejecting everything forever.

The circuit breaker's job is to protect a downstream service from
cascading failures. A circuit that never closes protects the downstream
service from all traffic, forever, including valid traffic after
recovery. This is not protection. It is an outage.

## The Fix

HalfOpen lets one request through as a probe. Exactly one. A CAS on a
`half_open_used` flag prevents the thundering herd:

```rust
State::HalfOpen => {
    if !self.half_open_used {
        self.half_open_used = true;
        true
    } else {
        false
    }
}
```

First caller in HalfOpen gets through. All others are rejected. If the
probe succeeds, `record_success()` resets to Closed and clears
`half_open_used`. If it fails, `record_failure()` pushes back to Open
and clears the flag for the next cooldown cycle.

```rust
pub fn record_success(&mut self) {
    if self.state == State::HalfOpen {
        self.state = State::Closed;
        self.failure_count = 0;
        self.half_open_used = false;
    }
}

pub fn record_failure(&mut self) {
    self.failure_count += 1;
    self.last_failure = Some(Instant::now());
    if self.state == State::HalfOpen {
        self.state = State::Open;
        self.half_open_used = false;
    }
}
```

The cooldown timeout triggers the Open → HalfOpen transition on the
next `allow()` call, not on a background timer. This means the circuit
doesn't advance state unless something is actually trying to send.

## Why the Bug Feels Right

The failure instinct says: something is broken, reject everything until
it's fixed. That's correct for Closed → Open. It's wrong for HalfOpen,
because HalfOpen IS the "check if it's fixed" mechanism.

If you write `State::HalfOpen => false`, you're saying "don't check if
it's fixed." The circuit will stay Open until the process restarts or
someone manually resets it. Meanwhile every request is rejected and the
downstream service, which may have recovered minutes ago, is never
tested.

The bug is indistinguishable from a correctly tripped circuit for a
naive observer. Both Open and broken-HalfOpen return false. The
difference only appears when the downstream recovers — Open eventually
transitions, broken-HalfOpen never does.

## Test for the Exit Condition

For every non-terminal state in a state machine, write a test that
exercises the transition out of it.

```rust
#[test]
fn circuit_half_open_success_closes() {
    let mut cb = CircuitBreaker::new(3, Duration::from_millis(10));
    for _ in 0..3 {
        cb.record_failure();
    }
    // wait for cooldown
    loop {
        if cb.allow() { break; }
        thread::sleep(Duration::from_micros(100));
    }
    assert_eq!(cb.state(), State::HalfOpen);
    cb.record_success();
    assert_eq!(cb.state(), State::Closed); // <-- tests the exit
}
```

This test fails with `State::HalfOpen => false` because `cb.allow()`
never returns true, so the loop never breaks. The deadlock is visible
in the test before it's visible in production.

The rule: HalfOpen without a probe is a dead state. Any non-terminal
state that has no exit condition is a dead state. Write the exit test
before the implementation.

## See Also

- `rsx-gateway/src/circuit.rs` - CircuitBreaker implementation
- `rsx-gateway/tests/circuit_test.rs` - HalfOpen transition tests
