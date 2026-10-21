# CLAUDE.md — rsx-cast

Local to `rsx-cast/`; the root `../CLAUDE.md` applies. The crate's docs stand
alone (§ Standalone), so this file names skills by name, never by a `../`
path.

## FROZEN — bugfixes only

rsx-cast is complete. It accepts bugfixes only: restoring documented or
intended behaviour (a CRC error, a crash, a wire-format mismatch, a NAK or
replay correctness fault). Anything else — new API, record type, signature or
behaviour change, refactor, dependency bump, performance rework — needs the
founder's explicit sign-off BEFORE code. Work in another crate that seems to
need an rsx-cast change stops and asks; the fix is almost always in the
caller (the SEQ-1 fan-out fix lived in matching and risk, not here).
Read-only findings go to `BUGS.md`.

Signed-off exceptions (2026-10-21): fix the two WAL bugs — a failed partial
`WalWriter::flush` whose retry rewrites the whole buffer, and `WalReader`
treating a mid-file CRC failure as EOF.

## Rules

- Zero runtime deps. `CastSender`, `CastReceiver`, `WalWriter`, `WalReader`,
  `ReplicationService` and `ReplicationConsumer` are synchronous. NEVER
  monoio, tokio or any executor, not optional, not behind a feature. The
  caller owns the socket and the wakeup; `CastReceiver` takes bytes and
  returns records.
- Names: casting is the UDP live path, replication is the WAL-over-TCP path.
  NEVER invent a third name.
- One copy per record on the cast path and one CRC per record: a record is
  framed once (one WAL seq, one CRC) and fanned to the WAL and every stream.
  There is no send path without a WAL.
- Loss handling: NAK with debounce on both sides; a reorder-buffer overflow
  reconnects and replays over TCP; it NEVER skips a gap silently. FAULTED is
  the NAK-exhausted fallback only.
- The hot WAL is a short retention tier; moving segments to an archive or
  deleting them is not the writer's job.
- Replication TLS is mandatory (snakeoil certs for dev); no plaintext arm.
- Default `rustfmt` applies here as everywhere; nothing is hand-formatted.
- Benches pin their cores and run with the cluster stopped; a bench that can
  hang on one lost loopback packet is a critical bug, not a flake.
- Consumers pin a release tag of this repo; a tag is never force-moved.

## README style (after `rtrb`)

`rtrb` (https://github.com/mgeier/rtrb) is the reference: a one-sentence
pitch on line two, orientation not an API dump, no badges, no marketing
("wait-free" is technical, "blazingly fast" is not), short sections,
alternatives cited generously (`compare/niche.md`, five links in the README),
lineage acknowledged (LBM → Aeron → MoldUDP64), an honest performance section
("loopback microbenches; run `cargo bench` yourself"), the licence block
aligned with `Cargo.toml`. The pitch says low-latency, batched WAL, replay
fallback on a NAK outside the buffer, multicast not pub/sub. It never says
"fault-tolerant", never praises or ties MoldUDP64 in the pitch (the
comparison lives in `compare/`), and never claims a publication or a
changelog that does not exist.

## Keeper sections — do not regress

rtrb documents a 200-line data structure; this crate documents a protocol
readers do not already know. Cut fluff, keep substance:
- "Why this exists", "Wire format" (the 16-byte header is load-bearing),
  "Guarantees" (what it does TODAY, caveated where a claim is pending), "When
  NOT to use this" (lossy WAN, O(N) cold retransmit, one producer per stream,
  no congestion control), "Requirements and assumptions" (a trusted, stable
  network), the specific bench numbers with their bench named, and the
  quick-start examples (the send loop is non-obvious).
- Every binding a quick-start constructs is used.
- Every cross-reference points at a file that exists.

## Standalone — no parent-relative paths

rsx-cast is the extractable artifact. Its `README.md`, `ARCHITECTURE.md` and
this file assume the reader has only this crate.
- NEVER `../` links; NEVER sibling-crate source references ("part of the
  wider rsx exchange" is fine). Full GitHub URLs are fine.
- The protocol is documented in `ARCHITECTURE.md`; there is no `specs/` copy
  here. A project-level fact the README needs is inlined or copied into
  `facts/`, never linked out.

## Numbers — source of truth

`cargo bench` is authoritative; `facts/*.md` records dated measured numbers;
`README.md` and `ARCHITECTURE.md` quote them with the bench name and a
p50/p99 pair. Two numbers for one thing are both labelled by bench variant
or one of them is wrong.
