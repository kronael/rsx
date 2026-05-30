# WAL is Wire is Stream

Most distributed systems have three representations of the same
data: an in-memory struct, a wire encoding, and a disk format.
Converting between them takes time — FlatBuffers costs around
150 ns per message, hand-rolled msgpack around 80 ns. Across
five hops (gateway → risk → matching → risk → gateway), that
adds up.

RSX removes the conversion by having only one representation.

## The single layout

Every exchange record — fills, BBO updates, order events — is a
`#[repr(C, align(64))]` Rust struct with explicit field order and
padding. The same bytes that sit in the matching engine's event
buffer are written verbatim to the WAL file, sent verbatim in a
UDP datagram, and streamed verbatim over TCP for replay. There is
no encoder. There is no decoder. There is a CRC32C check on the
payload, and that is all.

```rust
#[repr(C, align(64))]
pub struct FillRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub taker_user_id: u32,
    pub maker_user_id: u32,
    pub _pad0: u32,
    // ... order ids, price, qty, flags, padding to 64 bytes
}
```

This is the fill record in memory, on disk, on the wire, and in
the replay consumer's receive buffer. The pointer arithmetic is
a single `memcpy` into a socket or file buffer that the kernel
would have to do anyway.

WAL append writes a 16-byte header followed by the raw struct bytes.
The header carries a version byte (at offset 0 so receivers can
gate on it before reading anything else), record type, payload
length, and CRC32C of the payload. No vtable, no schema lookup,
no allocation.

Measured: 31 ns for `WalWriter::prepare` plus `append_framed`
(the Vec extend, no disk I/O). `FillRecord` encode: 23 ns.
Decode: 9 ns. The per-hop savings over a schema-based format are
real and cumulative.

## What this costs

The format is frozen at the struct layout level. You cannot
reorder fields. You cannot remove fields. The rule is:
append-only, pad-reuse.

Adding a field means filling a padding slot or extending the
struct (and bumping the size, which is its own coordination
cost). If a record type genuinely needs a breaking change, the
convention is a new `record_type` constant — `RECORD_FILL_V2`.
Old consumers ignore unknown record types and move on.

The version byte in the header is reserved for changes that
break the framing itself: header layout, CRC algorithm,
alignment promises. That kind of change requires stopping all
producers, upgrading all readers, and restarting. It has
happened once (when the version byte moved from offset 8 to
offset 0 in commit `64dda88`); the old V0 format was retired
and will not be read again.

Little-endian x86_64 and aarch64-LE only. There is a
compile-time check. Big-endian is not supported.

## The alternative rejected

The team tried FlatBuffers first. The benchmark result: 150 ns
per message. The conceptual result: three separate code paths
(in-memory struct, wire builder, file layout) that drift apart
when anyone adds a field and forgets to update all three.

The `repr(C)` approach trades schema flexibility for zero
translation overhead and a single source of truth. For a system
where the wire format, disk format, and in-memory format must
all agree anyway — or recovery breaks — the tradeoff is correct.

The one genuine hazard is that `repr(C)` struct layout depends on
the compiler respecting the declared field order and the explicit
padding. The `align(64)` and explicit `_pad` fields are load
bearing. Change the struct layout carelessly and the CRC will
catch the corruption, but only after data is already wrong. The
discipline required is: treat these structs like a wire protocol
specification, because they are one.

---

Deeper: [blog/12-deleted-serialization.md](../../blog/12-deleted-serialization.md),
[blog/dont-yolo-structs-over-the-wire.md](../../blog/dont-yolo-structs-over-the-wire.md),
[specs/2/4-cast.md](../../specs/2/4-cast.md),
[specs/2/48-wal.md](../../specs/2/48-wal.md)
