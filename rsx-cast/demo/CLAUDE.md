# demo/rsx-cast — the transport

How to demo `rsx-cast` (casting + WAL + replication). General method,
recording gotchas, and design rules: the `speed-demo` skill.

## The pitch
Two acts.

**Act 1 — flowing narrative.** Typed to stdout and left there — fills
top-down like a real session, no clears, real cursor trailing. Reading-pace
typing (~45 ms/char), stops at `.!?` only, a beat at paragraph breaks.
Colors on individual words of meaning, not sentences (mapping in Palette).

1. **"rsx-cast — move every byte fast, never lose one."**
2. **"Total reliability costs latency:"** a broker adds hops, and fsync on
   every byte costs milliseconds — a hundred times the network hop.
   ("a hundred" = 1 ms / ~9.3 µs hop ≈ 107×; NOT "a thousand". No
   "hand-rolled UDP" as a cost — rsx-cast IS hand-rolled UDP.)
3. **"If your servers never all die at once, don't pay for it."**
4. **"rsx-cast is as fast as it goes, and as reliable as it gets. How?"**
5. **"By making it minimal."** — multicast, not pub/sub; same bytes on
   wire/disk/replay; NAK recovers a live gap, the batched WAL is the
   fallback, off the critical send path.
6. **"One library, one binary: the WAL and the replay server can run in
   your process. Nothing to deploy."** ("can" — in-process is an option,
   not a requirement; a plain sidecar binary works too.)
7. **"Ties raw UDP. Beats TCP and QUIC."**

**Act 2 — the benchmark.** The one screen clear → centered "See for
yourself." (~2.6 s) → a 10 s packet-count race → held 15 s → CTA "Read
the code." / `github.com/kronael/rsx`. All in one `rich.Live` region.
The bordered panel counts packets moved in the elapsed time (count =
1/latency × elapsed, linear — a real counter, not an eased rate), with a
live "packets moved in N s" timer and latency per row; cast starred at
the end. Row colors saturate from DIM toward each hue as the race opens
up. On screen only the scope line ("2 pinned cores, 128B, loopback");
derivation (counts = 1/latency × 10 s, not a separate bench) and citation
stay here, not in the panel.

## Artifacts
- `pitch.py` — the whole recording. Python + `rich`, PEP 723 inline deps,
  run via `uv run pitch.py`. Hand-coded, no shared helper framework. All
  numbers cited (see Honesty) — the four-protocol comparison can't run
  live in one process.
- `Makefile` — `make rec` → `make gif`.
- `cast-live-opt.gif` — the tracked output (the postable artifact).
  The raw `.gif` and the `.cast` recording are gitignored build
  intermediates — regenerate with `make rec gif`.

## Regenerate
```
cd rsx-cast/demo
make rec   # asciinema, --cols 44 --rows 12
make gif   # agg --theme monokai --font-size 28, then gifsicle
```
`--cols`/`--rows` must match between `rec` and `gif`.

## Palette — "Cemani"
Project-wide palette, sampled from a black-rooster-in-spring photo
(Shutterstock 2160144679), lifted to UI-legible brightness. Playground and
TUI are meant to adopt it too (not yet done).

- `TEAL #57b0a3` — speed/good: cast + udp rows (they tie), the ★, speed words
- `GOLD #c9a24e` — brand/durability: tcp row, borders, claim openers, CTA
- `RUST #b0703f` — cost/worst: quic row, cost words
- `MOSS #9aad4c` — reserve, unused
- `FG #ece6d8` — body text
- `DIM #8f8672` — captions/scope

Background: `agg --theme monokai` (`#272822`). NOT `gruvbox-dark` (agg
renders it light) or `github-dark` (too cool); `--theme custom` is broken
in agg 1.9.0. Never invent a hue the palette lacks — tied rows share one.

## Honesty
All four numbers measured 2026-07-07, two paired runs each (full criterion
triples in `compare/README.md`):
- raw UDP ~9.0µs (`compare_all::raw_udp_128b`) — shown DIM + "floor":
  the unprotected reference, no reliability, not a competitor
- casting ~9.5µs (`cast_rtt_bench`) — ★ = fastest reliable transport.
  cast and udp medians swap run-to-run (statistical tie; casting = UDP +
  ~26ns userspace, so the floor edging it by ~0.5µs is honest noise)
- TCP_NODELAY ~15.2µs (`compare_all::tcp_nodelay_128b`)
- Quinn/QUIC ~36.3µs (`compare_all::quinn_persistent_128b`)

Run non-KCP rows via Criterion name filters (the KCP warmup panic,
`BUGS.md` BENCH-KCP-FLUSH-NEEDUPDATE, fires only when KCP is selected).
2 pinned cores, 128B record, loopback — not the ~1.1ms cross-process
figure (`ARCHITECTURE.md`). Network transports only; shared-memory IPC
(Aeron IPC, Chronicle) is a different category, not shown.

## MoldUDP64 — off-screen deliberately
MoldUDP64 (`compare_moldudp64.rs`) ties or edges casting on raw speed
(~8.6µs vs ~9.3µs) — showing it undercuts the pitch without context that
fits no card line. The differentiator is durability: Mold's retransmit
source is an external request server and its archive a separate service
(TotalView Glimpse); casting's WAL is embedded, same bytes as wire/replay.
They tie because both are syscall-dominated (~3.6-4µs sendto): Mold's
framing computes no checksum, casting adds CRC32C — tens of ns, noise
against the floor. Detail: `compare/moldudp64.md`.

## Do NOT
- "fastest" only scoped "over the network" — shared-memory IPC beats it.
- Not "fault-tolerant" — NAK recovery + WAL replay; say exactly that.
- Never swap in a bar number without re-running the bench; update the
  `PROTOS` tuple and the Honesty list together.
