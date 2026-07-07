# demo/rsx-cast — the transport

How to demo `rsx-cast` (casting + WAL + replication). Read the `speed-demo`
skill first for the general method.

## The pitch
X/Twitter-feed friendly. Two acts, NOT a slideshow:

**Act 1 — flowing narrative.** Typed to stdout and LEFT there — it flows
top-down and fills the screen like a real terminal session, NO clear and
NO re-centering between claims (that "reshow" reads as slides and kills the
terminal illusion). The real terminal cursor trails the typing. Organic
pacing: reading-speed typing (~45 ms/char) with a stop at sentence-ending
`.!?` only (not commas/colons/every word) and a beat at paragraph breaks.
Colors are applied to individual words of meaning, NOT whole sentences:
speed words teal, durability/brand words gold, cost words rust, connective
prose warm off-white. The arc is a tension → resolution → mechanism:

1. **"rsx-cast — move every byte fast, never lose one."** (the promise)
2. **"Reliability always costs you something:"** a broker and its hops, or
   hand-rolled UDP you patch yourself (the tension; costs in rust)
3. **"rsx-cast is as fast as it goes, and as reliable as it gets. How?"**
   (the resolution)
4. **"By making it minimal."** — multicast, not pub/sub; same bytes on
   wire/disk/replay; NAK recovers a live gap, the batched WAL is the
   fallback, off the critical send path (the mechanism)
5. **"Ties raw UDP. Beats TCP and QUIC."** (the punch, in gold)

**Act 2 — the benchmark.** The ONE screen clear. Opens on a weighty centered
callout **"See for yourself."** (held ~2.6s), then the numbers climb in, then
the CTA — all in a single `rich.Live` region so callout → results → CTA swap
in place with NO further clears. The combined data panel (bordered — the one
place a box belongs, since it's structured data, not prose) shows throughput
(round-trips/sec, counting up over 10s, bigger = better/intuitive) and
latency (µs) **side by side per row**, cast starred as best, held 15s once
landed. On screen only the scope line ("2 pinned cores, 128B, loopback") —
the derivation (throughput = 1/latency, not a separate bench) and the
citation (compare/README.md) live HERE, per founder: no caption clutter in
the panel. Ends on ONE call to action: "Read the code." /
`github.com/kronael/rsx`.

casting and raw UDP share the same **teal** (they genuinely tie); TCP is
**gold**, QUIC is **rust** — a best→worst gradient using only real palette
hues (see Palette below), no invented fourth color.

MoldUDP64 is deliberately NOT in this on-screen comparison — see the
MoldUDP64 section below for why (it ties/beats casting on raw speed; the
real differentiator is durability, not speed, and that nuance doesn't fit
a card line).

## Artifacts in this folder
- `pitch.py` — the whole recording: plain-text cards + a bordered
  throughput/latency data panel + a closing CTA. Python + `rich` (`Text`+
  `Padding` for the cards, `Panel` only for the data panel, `Live` for both
  the typewriter reveal and the count-up) — PEP 723 inline deps, run via
  `uv run pitch.py` (no separate install step, no venv to manage). Written
  directly (no generic reusable helper framework — see `speed-demo` skill's
  note on why per-project demo scripts stay hand-coded). Single script, no
  separate live-vs-scripted split (unlike book/matching/risk) because the
  four-protocol comparison can't currently be run live in one process (see
  honesty note below) — every number is cited from the dated report.
- `Makefile` — `make rec` (asciinema) → `make gif` (agg + gifsicle).
- `cast-live.cast` / `cast-live-opt.gif` — the recorded output (generate
  with `make rec gif`; not committed until generated).

## Regenerate
```
cd rsx-cast/demo
make rec   # records pitch.py (via uv run) through asciinema, --cols 44 --rows 12 (half-height, font-size 28 = hi-DPI glyphs)
make gif   # renders (agg --theme monokai) + optimizes the GIF
```
`--cols`/`--rows` MUST match between `rec` and `gif` exactly (see `speed-demo`
skill's recording gotcha) — if you change one, change both.

## Palette — "Cemani" (the black-rooster-in-spring palette)
**This is the intended project-wide palette** — sampled from the reference
photo (a black Ayam-Cemani rooster with iridescent teal/green feather sheen,
against warm olive-gold spring bokeh, moss grass, and brown earth/tree-trunk;
Shutterstock id 2160144679). Hues are faithful to the photo, lifted to
UI-legible brightness on a warm-dark base. Playground and TUI are meant to
adopt these too (larger separate change — not yet done there).

Sampled hues (via PIL on the real image — dominant + most-saturated
families), then UI-tuned:
- `TEAL #57b0a3` — iridescent feather sheen, the signature. **cast + udp**
  (they tie), the `★ best` star.
- `GOLD #c9a24e` — warm olive-gold bokeh (the dominant background hue).
  Primary accent: **tcp** (middle), every panel border, every claim opener,
  the CTA.
- `RUST #b0703f` — warm earth / tree-trunk brown. The "cost"/worst: **quic**,
  and the "reliability costs you something" line.
- `MOSS #9aad4c` — spring grass green (held in reserve; not currently placed).
- `FG #ece6d8` — warm off-white highlight — body text.
- `DIM #8f8672` — muted warm grey-brown — captions/scope.

Base (background) is `agg --theme monokai` (`#272822`, warm dark) — NOT
`gruvbox-dark` (agg 1.9.0 renders that with gruvbox's LIGHT cream bg, a real
quirk), NOT `github-dark` (too cool for this warm palette). `--theme custom`
is broken in agg 1.9.0 (see `speed-demo` skill), so an exact `#0b0e11`-class
hex isn't available — monokai is the closest warm-dark built-in.

The comparison gradient (teal best → gold → rust worst) uses only these
hues; cast+udp sharing teal is intentional (they genuinely tie). Do NOT
invent a hue the palette doesn't define to give a tied row its own color.

## Honesty (on screen + here)
**All four numbers are currently cited from `compare/README.md`**, not
live-measured in this recording:
- casting ~9.3µs (`cast_rtt_bench`, 2026-07-01) — live re-run is currently
  blocked, see `BUGS.md` `CAST-RTT-BENCH-HANGS-AFTER-SEND-REMOVAL`
- raw UDP ~9.9µs (`compare_all::raw_udp_128b`, 2026-07-01)
- TCP_NODELAY ~14µs (`compare_all::tcp_nodelay_128b`, 2026-05-24)
- Quinn/QUIC ~37µs (`compare_all::quinn_persistent_128b`, 2026-05-24)

`compare_all` cannot currently produce all four live in one run either way:
it panics on KCP warmup before reaching quinn/tcp (`BENCH-KCP-FLUSH-
NEEDUPDATE`), so even a "live" run of that harness would only get raw_udp.
2 pinned cores (client + server), one 6-core box, 128B record (matches
`FillRecord`), loopback. Not a cross-process/production number — see
`ARCHITECTURE.md` for the ~1.1ms cross-process figure.

The comparison set is network transports only (casting, raw UDP, TCP,
QUIC) — shared-memory IPC (Aeron IPC, Chronicle Queue) is deliberately
excluded, different category, not shown.

## MoldUDP64 — deliberately NOT in the on-screen pitch, explained here instead
MoldUDP64 (our clean-room reimpl, `compare_moldudp64.rs`) ties or slightly
beats casting on raw speed (fresh run: ~8.6µs vs casting's ~9.3µs cited
figure) — do NOT put this comparison on screen; it undercuts the pitch
without the context below, which doesn't fit in a 5-7-word card line. Keep
it here and in `compare/moldudp64.md` only.

The actual differentiator is durability, not speed: MoldUDP64's retransmit
source is an external "request server" and its durable archive is a
**separate service** (Nasdaq's TotalView Glimpse) — not part of the
protocol library itself (`compare/moldudp64.md:107,110`). Casting's WAL is
embedded in the same library, same bytes as the wire/replay stream.

Why they tie despite casting doing more work: `frame_packet` in
`compare_moldudp64.rs:72-91` computes no checksum at all (pure memcpy —
session id + seq + msg_count + length-prefix + payload). Casting's
`Framed::pack` (`src/wal.rs`) does the same memcpy plus a CRC32C over the
payload. Both are dominated by the `sendto`/`recvfrom` syscall (~3.6-4µs
per `facts/cast-vs-udp-overhead.md`'s send-breakdown), so the CRC's tens
of nanoseconds are noise against that floor — casting ties while doing
strictly more (checksum + durability bookkeeping), because the extra work
is free relative to the syscall cost.

## Do NOT
- Do NOT re-add "fastest" without scoping it to "over the network" — Aeron
  IPC and Chronicle Queue beat casting's number, just not as network
  transports.
- Do NOT call this "fault-tolerant" — no replica failover, no consensus.
  It's NAK recovery + WAL replay, say exactly that.
- Do NOT swap a bar's number to live-measured without re-running the bench
  yourself first (per `speed-demo`: never a remembered number). Update the
  `PROTOS` tuple in `pitch.py` and this file's honesty note together, only
  once you have a fresh number in hand.
