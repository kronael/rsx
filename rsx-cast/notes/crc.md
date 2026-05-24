# CRC choice — why CRC32 IEEE, not CRC32C

rsx-cast computes a CRC32 per record (16-byte header + payload).
This note records the choice of CRC variant and why we're not
switching to the faster Castagnoli polynomial despite the
free-on-x86 single-instruction path.

## What we use today

`rsx-cast/src/encode_utils.rs::compute_crc32` uses the
[`crc32fast`](https://docs.rs/crc32fast) crate — CRC32 IEEE
(zlib polynomial `0xEDB88320` reversed, equivalently
`0x04C11DB7` forward). Same polynomial as gzip, PNG, and
Ethernet FCS. crc32fast does runtime feature detection and
takes the PCLMULQDQ-folding path on x86_64 from Westmere
onward (2010+), giving ~10–20 GB/s.

For a 128-byte payload that's ~6–12 ns per record CRC.

## CRC32C exists and is faster

CRC32C (Castagnoli polynomial `0x1EDC6F41`) is what SCTP, iSCSI,
btrfs, and ext4 metadata use. SSE4.2 (Nehalem, 2008+) added a
hardware `crc32` instruction that hashes 8 bytes in ~1 cycle.
Via `core::arch::x86_64::_mm_crc32_u64` you get ~30–50 GB/s —
roughly 3× crc32fast's PCLMULQDQ-folded CRC32 IEEE path.

At our payload sizes (128 B Fill, ≤256 B WAL slot):

| Variant | Cycles for 128 B | ns @ 3 GHz |
|---|---:|---:|
| `crc32fast` (PCLMULQDQ, CRC32 IEEE) | ~24 | ~8 |
| `_mm_crc32_u64` (CRC32C) | ~16 | ~5 |

3 ns saved per record. Worth knowing; not worth chasing.

## Why we don't switch

1. **Wire incompatible.** All existing WAL files become
   unreadable. CRC bytes 8..12 of `WalHeader` would change
   meaning. A flag day, not a soft migration. The crate is
   pre-1.0 and we *could* break, but every other v0.x bump
   has been pure-Rust-symbol or layout-with-version-byte.
   Switching CRC polynomial is structurally identical to
   changing the wire format.

2. **The 3 ns is invisible.** `CastSender::send` is ~4 µs;
   `sendto` itself accounts for 99% of it (see
   [`cast_send_breakdown_bench`](../benches/)). `WalWriter::append`
   is 31 ns p50 and CRC is roughly 6 of those. Saving 3 ns
   inside a 31 ns operation, on a path that's not even the
   hot path (sendto dominates), is below measurement noise.

3. **Detection quality is equivalent at our lengths.** Both
   IEEE and Castagnoli polynomials have Hamming distance 4
   at our record sizes. Castagnoli is technically slightly
   better at very long messages (>2 KB), but we never see
   those: WAL payloads cap at 256 bytes per slot, and the
   only larger blobs go through the WAL flush+fsync path
   where the 3 ns CRC delta is dwarfed by 24–651 µs of
   disk I/O. CRC choice is performance, not correctness.

4. **The detection failure modes we care about are
   end-to-end, not polynomial-specific.** A flipped bit in
   memory between encode and flush is caught by either
   polynomial. A corrupted file region produces a CRC
   mismatch with either. We're not protecting against a
   structured-error class (e.g. burst errors of a specific
   length) that CRC32C handles better than CRC32 IEEE.

## When this becomes worth revisiting

- A wire format major version is being bumped for other
  reasons (multicast framing, payload-length > u16,
  segmented records, header rework). Bundle the CRC switch
  with it so we pay the migration cost once.
- Average payload size grows >1 KB. The CRC32C lead widens
  with length; it's ~10× at 4 KB, not 3×.
- We move CRC out of `send` into a hot inner loop where 3 ns
  visibly matters. Not the case today — sendto is the wall.

## Alternatives surveyed

| Crate / approach | Polynomial | SIMD | Throughput | Notes |
|---|---|---|---:|---|
| `crc32fast` 1.4 | IEEE | PCLMULQDQ (x86), NEON (aarch64) | 10–20 GB/s | What we use |
| `crc32c` 0.6 | Castagnoli | SSE4.2 `crc32` instruction | 30–50 GB/s | Wire-incompatible |
| Hand-rolled `_mm_crc32_u64` loop | Castagnoli | yes | ~50 GB/s | Same as `crc32c` crate, just no dep |
| `crc-any` | any (table-driven) | no | ~200 MB/s | Avoid on hot paths |
| Slice-by-8 / slice-by-16 (table-driven, no SIMD) | IEEE or C | no | 1–3 GB/s | Fallback for non-SIMD targets; not faster than PCLMULQDQ |

## "Defer CRC to flush" — considered, rejected

The alternative to switching polynomial is moving the
compute off the per-record path entirely: producer encodes
without CRC, writer fills it in at flush time over the
whole batch. This trades:

- ~6 ns saved per `send` call
- a window where the in-flight buffer has no CRC
- coupling between the encoder and the writer (the writer
  has to know payload boundaries to compute per-record
  CRCs)

We keep the per-record CRC. End-to-end protection from the
producer's call site to the reader's call site is worth
the 6 ns, especially because CRC is not the dominant cost
in any path we care about.

## Compiler flags worth knowing

We don't currently set `target-cpu=native` in
`.cargo/config.toml`. Setting it would let LLVM emit the
`crc32` instruction inline for SSE4.2+ targets *if* we were
on CRC32C — but we're not, so no effect. Adding
`-C target-feature=+pclmulqdq` is also a no-op because
crc32fast already runtime-detects and uses it.

The only flag that would change behavior here is
`-C target-cpu=native` paired with switching to CRC32C, and
we've already excluded that.

## References

- [`crc32fast` crate docs](https://docs.rs/crc32fast) —
  PCLMULQDQ implementation and runtime feature detection.
- [`crc32c` crate docs](https://docs.rs/crc32c) — SSE4.2
  hardware instruction path.
- Intel, ["Fast CRC Computation Using PCLMULQDQ Instruction"
  (2009)](https://www.intel.com/content/dam/www/public/us/en/documents/white-papers/fast-crc-computation-generic-polynomials-pclmulqdq-paper.pdf)
  — the white paper crc32fast's algorithm derives from.
- Castagnoli, Bräuer, Herrmann, ["Optimization of Cyclic
  Redundancy-Check Codes with 24 and 32 Parity Bits"
  (IEEE Trans. on Comm., 1993)](https://ieeexplore.ieee.org/document/231911) —
  original derivation of the CRC32C polynomial.
- [Stephan Brumme, "Fast CRC32"](https://create.stephan-brumme.com/crc32/)
  — readable comparison of slice-by-N, PCLMULQDQ, and
  hardware-instruction approaches.
- `cast_send_breakdown_bench.rs` — per-step attribution of
  the ~4 µs `CastSender::send`, confirming sendto is 99%.
- `wal_bench.rs` — `WalWriter::append` at 31 ns p50; CRC is
  a small share.
