# Backpressure, Not Drop

When a bounded buffer fills, every queue-based system faces the
same choice: stall the producer or drop the message. RSX always
stalls.

## Why dropping is wrong

A dropped fill is an invisible fill. The accounting diverges
silently. Prometheus increments a counter that no one watches
at 3 AM. Three days later a user files a support ticket about
a balance discrepancy. By then the fills that were dropped
cannot be reconstructed.

A stall is visible immediately. Latency spikes at p99. The
load balancer's health check times out. The on-call engineer
gets paged. The problem is surfaced at the moment it occurs,
not days later.

The system can tolerate a brief stall. It cannot tolerate
invisible data loss.

## Where the stalls are

The WAL buffer is bounded. If the buffer fills — because the
disk is slow and fsyncs are taking longer than 10 ms — the
matching engine's `append` call returns `WouldBlock` rather
than dropping the record. The engine handles this by flushing
immediately (blocking on fsync) and then retrying. While the
engine waits for the disk, it processes no new orders. The
gateway queues fill. Clients see elevated latency. The on-call
team investigates the disk.

SPSC rings are bounded by construction — `rtrb` ring buffers
are fixed-size at creation. When a ring is full, the producer
either spins waiting for space or sets a backpressure flag that
suppresses further sends until the consumer drains the ring.
Which behavior is chosen depends on the ring's criticality:
the persist ring (risk → Postgres write-behind) stalls the hot
path on full; BBO and mark-price rings drop newest on full
and log a warning, because stale BBO is replaced by the next
update anyway.

## Small buffers surface problems early

There is a temptation to make buffers large to absorb bursts.
Large buffers hide problems. A 10 GB WAL buffer absorbs 100
million events before backpressure fires; by the time the
latency spike reaches monitoring, the system has accumulated
10 seconds of lag. A bounded buffer that fills in milliseconds
fires the alert before users notice.

The WAL buffer capacity is `max(2 × file_size, 256 KB)`. For
the default 64 MB WAL file size, that is 128 MB — enough to
absorb a burst, not enough to hide a sustained slow disk.

## The cost of honesty

Stalling the producer means stalling everything downstream.
If the WAL flush takes 50 ms because the SSD is struggling,
matching pauses for 50 ms. New orders are not accepted. This
is correct behavior — accepting orders during a flush stall
would accumulate a queue of unacknowledged intents that would
be lost if the process crashed during the stall.

The SLA is: if we accept your order, we will not lose your
fill. We do not promise we will always accept your order.

---

Deeper: [blog/15-backpressure-or-death.md](../../blog/15-backpressure-or-death.md),
[specs/2/48-wal.md](../../specs/2/48-wal.md)
