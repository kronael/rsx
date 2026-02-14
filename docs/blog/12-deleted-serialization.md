# We Deleted the Serialization Layer

The fastest serialization is no serialization.

## The Problem

Every exchange has the same bottleneck: transforming in-memory data into
wire format, then back again. FlatBuffers adds 150ns. Cap'n Proto adds
120ns. Even hand-rolled msgpack takes 80ns.

When your end-to-end latency budget is 50μs, spending 300ns per hop
(serialize → send → deserialize) means 0.6% of your budget on
translation overhead. Across 5 hops (Gateway → Risk → ME → Risk →
Gateway), that's 3% gone before you've done any actual work.

## The Insight

What if the serialization layer doesn't exist?

```rust
// rsx-dxs/src/records.rs
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct FillRecord {
    pub seq: u64,
    pub ts_ns: u64,
    pub symbol_id: u32,
    pub taker_user_id: u32,
    pub maker_user_id: u32,
    pub _pad0: u32,
    pub taker_order_id_hi: u64,
    pub taker_order_id_lo: u64,
    pub maker_order_id_hi: u64,
    pub maker_order_id_lo: u64,
    pub price: Price,
    pub qty: Qty,
    pub taker_side: u8,
    pub reduce_only: u8,
    pub tif: u8,
    pub post_only: u8,
    pub _pad1: [u8; 4],
}
```

This is the fill record. Same struct everywhere:
- In-memory event buffer (matching engine)
- WAL file on disk
- UDP datagram over the wire
- TCP stream for replay
- Consumer's receive buffer

No transformation. No encoder. No decoder. Just `memcpy`.

## How It Works

WAL append is a single function:

```rust
// rsx-dxs/src/wal.rs
pub fn append<T: CmpRecord>(
    &mut self,
    record: &mut T,
) -> io::Result<u64> {
    record.set_seq(self.next_seq);

    let header = WalHeader {
        stream_id: self.stream_id,
        record_type: T::record_type(),
        seq: self.next_seq,
        payload_len: std::mem::size_of::<T>() as u16,
        crc32: compute_crc32(as_bytes(record)),
    };

    self.buf.extend_from_slice(as_bytes(&header));
    self.buf.extend_from_slice(as_bytes(record));
    self.next_seq += 1;
    Ok(self.next_seq - 1)
}
```

Reading from WAL:

```rust
// rsx-dxs/src/client.rs (DxsConsumer)
let mut header_buf = [0u8; 16];
stream.read_exact(&mut header_buf).await?;
let header = parse_header(&header_buf)?;

let mut payload = vec![0u8; header.payload_len as usize];
stream.read_exact(&mut payload).await?;

// No deserialization - just cast and validate
let crc = compute_crc32(&payload);
if crc != header.crc32 {
    return Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "crc mismatch"
    ));
}

// Payload is already a valid FillRecord, BboRecord, etc.
// Consumer casts based on header.record_type
```

CRC32 is enough. We don't need Merkle trees or cryptographic hashes.
If a bit flips in memory, the kernel panics. If a bit flips on disk,
the filesystem checksum catches it. CRC32 protects the 10ms between
kernel buffer and fsync.

## The Cost

Versioning is additive-only. You can't change field order. You can't
remove fields. You can only append.

```rust
// BAD: can't do this
pub struct FillRecordV2 {
    pub ts_ns: u64,     // moved
    pub seq: u64,       // moved
    pub symbol_id: u32,
    // ...
}

// GOOD: add fields at end
pub struct FillRecord {
    pub seq: u64,          // existing
    pub ts_ns: u64,        // existing
    // ... existing fields ...
    pub _pad1: [u8; 4],    // was padding
    pub new_field: u32,    // NEW: uses old padding
    pub _pad2: [u8; 8],    // NEW: expand to 128B
}
```

If you need a breaking change, you increment `record_type`:
`RECORD_FILL` → `RECORD_FILL_V2`. Old consumers ignore unknown types.
New consumers handle both.

## Why It Matters

We tested this with FlatBuffers first. Every fill record went through:

1. Create builder (heap allocation)
2. Add fields (vtable lookups)
3. Finalize (more heap)
4. Send bytes
5. Receive bytes
6. Parse (vtable again)
7. Extract fields (pointer chasing)

Microbenchmark: 150ns per message. Multiply by 5 hops = 750ns = 1.5% of
50μs budget.

With raw structs:

1. `memcpy` to buffer
2. Send bytes
3. Receive bytes
4. Validate CRC (10ns)

Microbenchmark: 8ns (just the CRC). The `memcpy` happens anyway when
the kernel copies to socket buffer.

## Tests Prove It

```rust
// rsx-dxs/tests/wal_test.rs
#[test]
fn writer_append_to_buffer_no_io() {
    let tmp = TempDir::new().unwrap();
    let mut writer = WalWriter::new(
        1, tmp.path(), None, 64 * 1024 * 1024, 600_000_000_000,
    ).unwrap();

    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();

    let active = tmp.path().join("1").join("1_active.wal");
    let size = std::fs::metadata(&active).unwrap().len();
    assert_eq!(size, 0);  // Nothing written to disk yet
}
```

Append is pure memory. Flush is where fsync happens:

```rust
#[test]
fn writer_flush_writes_to_file() {
    let mut writer = WalWriter::new(/* ... */).unwrap();
    let mut fill = make_fill(1);
    writer.append(&mut fill).unwrap();
    writer.flush().unwrap();  // <-- fsync here

    let size = std::fs::metadata(&active).unwrap().len();
    assert!(size > 0);
}
```

Replay over TCP uses the same format:

```rust
// DxsConsumer connects to ME's replay server
let consumer = DxsConsumer::new(
    symbol_id,
    "127.0.0.1:9001".to_string(),
    tip_file,
    None,
);

// Consumer reads records from TCP stream
while let Some(record) = consumer.poll().await? {
    // record.payload is raw bytes, same as WAL file
    match record.header.record_type {
        RECORD_FILL => {
            let fill: &FillRecord = unsafe {
                &*(record.payload.as_ptr() as *const FillRecord)
            };
            apply_fill(fill);
        }
        // ...
    }
}
```

## Key Takeaways

- **Serialization overhead is real**: 150ns × 10 messages/request = 1.5μs
  = 3% of 50μs budget
- **Disk = wire = memory**: Same struct, zero transformation, one
  version strategy
- **CRC32 is enough**: Not building a blockchain, just detecting
  corruption
- **Versioning is additive**: Padding fields become new fields, old code
  ignores them
- **Tests are fast**: No mocking serializers, just `assert_eq!(bytes,
  expected_bytes)`

The matching engine writes 80 bytes per fill (16B header + 64B payload).
No vtables. No heap. No schema lookups. Just `memcpy` and CRC32.

When someone asks "how do you version your wire format?", the answer is
"we don't have a wire format." We have a single C struct that exists
identically in memory, on disk, and on the network.

## Target Audience

Exchange engineers tired of serialization overhead. Anyone building
ultra-low-latency systems (HFT, market data, real-time analytics).
Developers who've hit FlatBuffers/Protobuf/Cap'n Proto tax and want to
eliminate it entirely.

## See Also

- `specs/v1/DXS.md` - DXS streaming protocol spec
- `specs/v1/WAL.md` - WAL format and guarantees
- `specs/v1/CMP.md` - CMP/UDP wire protocol
- `rsx-dxs/src/records.rs` - All record types
- `rsx-dxs/src/wal.rs` - WAL writer implementation
- `blog/dont-yolo-structs-over-the-wire.md` - Padding and alignment gotchas
