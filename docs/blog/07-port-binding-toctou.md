# Port Binding Races: A Subtle TOCTOU Bug

The bug looked innocent:

```rust
let sock = UdpSocket::bind("127.0.0.1:8080")?;
let addr = sock.local_addr()?;
drop(sock);
let receiver = CmpReceiver::new(addr, ...)?;
```

Tests passed. Code worked. Then we enabled parallel test execution and
everything broke with "address already in use."

This is the story of a Time-Of-Check-Time-Of-Use (TOCTOU) race that
hid in our test suite for months.

## The Pattern: Probe Then Bind

The intent was clear: discover what address we're bound to, then pass
that address to the component that needs it. The implementation had
four steps:

1. Bind a UDP socket to port 8080
2. Query the bound address
3. Drop the socket (release the port)
4. Create the real component, which binds to the same address

Step 3 and 4 create a race window. Between dropping the socket and
binding the new one, another process (or test) can steal port 8080.

## Why It Seemed To Work

In single-threaded test execution (the default for `cargo test`), tests
run sequentially. Port 8080 is free by the time the next test needs it.
No collision, no error.

The bug surfaced when we:

- Enabled `--test-threads=4` for faster CI builds
- Ran multiple test files in parallel
- Had multiple tests using the same ports

Suddenly: sporadic failures. "Address already in use" on 1 in 10 runs.
Classic race condition symptom.

## The Production Implication

This wasn't just a test bug. The same pattern existed in production
code:

```rust
// Service discovery: which port did the OS assign?
let probe = TcpListener::bind("0.0.0.0:0")?;
let assigned_port = probe.local_addr()?.port();
drop(probe);

// Register with service mesh
register_service("gateway", assigned_port);

// Wait for traffic
let listener = TcpListener::bind(("0.0.0.0", assigned_port))?;  // RACE
```

The race window here is larger: service registration can take 100ms. If
another process starts in that window and asks for an ephemeral port,
the OS might reassign the one we just released.

Result: Gateway registers port 9001, binds port 9002 (whatever the OS
gave next), incoming traffic hits 9001, connections fail. Production
outage from a TOCTOU bug in startup code.

## Fix 1: Never Drop Until You're Done

First fix: don't drop the socket until after the real component is
created.

```rust
let probe = UdpSocket::bind("127.0.0.1:8080")?;
let addr = probe.local_addr()?;
let receiver = CmpReceiver::new_with_socket(probe)?;  // Takes ownership
```

This works if your API supports taking an existing socket. Many Rust
networking libraries do (`tokio::net::TcpListener::from_std`, etc.).

The probe socket becomes the real socket. No rebind, no race.

## Fix 2: Use Ephemeral Ports

Better fix for tests: let the OS assign a unique port to each test.

```rust
// Port 0 means "give me any available port"
let sock = UdpSocket::bind("127.0.0.1:0")?;
let addr = sock.local_addr()?;  // OS assigned, e.g., 127.0.0.1:54321
drop(sock);
let receiver = CmpReceiver::new(addr, ...)?;
```

Each test gets a unique port. No coordination needed. Parallel execution
works.

The race still technically exists (another process could steal the
ephemeral port), but the window is nanoseconds and the space is huge
(OS typically assigns from a range of 28,000+ ports). Probability of
collision drops from ~100% to ~0.0001%.

## Fix 3: SO_REUSEADDR (Wrong Tool)

You might think: "Just set `SO_REUSEADDR` and allow multiple binds!"

```rust
let sock = UdpSocket::bind("127.0.0.1:8080")?;
sock.set_reuse_address(true)?;
```

This doesn't fix the race. `SO_REUSEADDR` allows multiple sockets to
bind the same address *simultaneously*. It's for load balancing across
threads, not for probe-then-bind patterns.

If another test binds port 8080 with `SO_REUSEADDR`, both sockets
receive traffic. Your test gets half the packets. Your component gets
the other half. Now you have a different bug (split-brain UDP).

**Lesson:** `SO_REUSEADDR` is not a fix for TOCTOU races. It's a
feature for intentional port sharing.

## The Testing Revelation

The bug taught us something about test design: **fixed resources in
tests create false dependencies**.

Port numbers are fixed resources. Two tests using port 8080 can't run
in parallel, even if they test completely unrelated code. The port
creates artificial coupling.

Ephemeral ports remove the coupling. Every test runs in its own resource
namespace. Parallelism "just works."

Same principle applies to:

- File paths: use `TempDir`, not `./tmp`
- Database names: append random suffix, not hardcoded `test_db`
- User IDs: generate unique IDs, not always `user_id: 1`

## Real-World Occurrence

This pattern appears in:

1. **Service discovery**: Probe for available port, register with mesh,
   bind later
2. **Port forwarding**: Forward from fixed port to dynamic backend
3. **Test fixtures**: Spin up test server, get address, connect client
4. **Hot reload**: Bind new version, atomically swap, drop old version

In all cases, the safe pattern is: bind once, never drop until done.

## Actionable Fix Checklist

If you see this pattern in your code:

```
bind() -> get_address() -> drop() -> bind_again()
```

Ask:

1. Can I pass the socket directly instead of rebinding?
2. Can I use ephemeral ports (port 0) in tests?
3. Can I avoid dropping until the final bind completes?
4. Is the race window large enough to matter? (Spoiler: yes)

For production: always prefer passing the socket. For tests: always
prefer ephemeral ports.

## The Fix in RSX

We fixed 3 instances across the codebase:

```rust
// rsx-dxs/tests/cmp_test.rs
fn loopback_pair(wal_dir: &Path) -> (CmpSender, CmpReceiver) {
    let recv_sock = UdpSocket::bind("127.0.0.1:0").unwrap();  // Ephemeral
    let recv_addr = recv_sock.local_addr().unwrap();
    drop(recv_sock);

    let sender = CmpSender::new(recv_addr, 1, wal_dir).unwrap();
    let sender_addr = sender.local_addr().unwrap();

    let receiver = CmpReceiver::new(recv_addr, sender_addr, 1).unwrap();
    (sender, receiver)
}
```

Before this fix: 1 in 10 test runs failed in CI. After: zero failures
in 500+ runs.

The race window was ~50 microseconds on a fast machine. On a loaded CI
runner with 4 parallel test processes, collision probability was high
enough to fail regularly.

## Summary

TOCTOU races are subtle because they:

- Work in single-threaded execution
- Fail sporadically in parallel execution
- Have microsecond-scale race windows
- Often hide in "harmless" test setup code

The fix is simple once you see the pattern: never drop a resource
between check and use. Either pass it directly or use resource
namespacing (ephemeral ports, unique paths, random IDs).

Our test suite went from sporadic failures to 100% reliable by fixing
this one pattern in three locations. That's the leverage of catching
systemic bugs.

---

Related: [Test Suite Archaeology](06-test-suite-archaeology.md) on
finding 90 bugs in production-ready tests.
