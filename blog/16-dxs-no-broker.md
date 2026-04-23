# DXS: Every Producer Is the Broker

No Kafka. No NATS. Producers serve their own WAL over TCP.

## The Problem

Standard event streaming architecture:

```
Producer → Kafka → Consumer
```

Kafka sits between producer and consumer:
- Producers write to Kafka (network hop)
- Kafka persists to disk (fsync)
- Consumers read from Kafka (network hop)
- Kafka manages offsets, partitions, rebalancing

Latency: producer writes event → consumer receives event = 5-10ms
(Kafka p50). Add replication: 10-20ms.

For a matching engine producing 100K fills/sec, that's 100K messages ×
10ms lag = 1000 seconds of accumulated lag per second. Consumers are
always behind.

## The Insight

The producer already has a WAL. The producer already persists to disk.
The producer already assigns sequence numbers.

**Why not let consumers read directly from the producer's WAL?**

```
Producer (WAL on disk) ← TCP ← Consumer
```

No broker. No middleman. Consumer connects to producer, requests
`seq=1234`, producer seeks WAL file and streams records.

## DXS Protocol

DXS = **D**ata e**X**change **S**treaming. Not a broker. A protocol.

Producer runs a replay server:

```rust
// rsx-dxs/src/server.rs (simplified)
pub struct DxsReplay {
    stream_id: u32,
    wal_dir: PathBuf,
    listener: TcpListener,
}

impl DxsReplay {
    pub async fn serve_one_client(&self) -> io::Result<()> {
        let (mut stream, addr) = self.listener.accept().await?;
        info!("dxs client connected: {}", addr);

        // Read replay request (16 bytes)
        let mut req_buf = [0u8; 16];
        stream.read_exact(&mut req_buf).await?;
        let req: ReplayRequest = parse_request(&req_buf)?;

        // Find WAL file containing req.start_seq
        let files = list_wal_files(&self.wal_dir, self.stream_id)?;
        let mut file = open_file_for_seq(&files, req.start_seq)?;

        // Stream records from start_seq onward
        loop {
            match read_next_record(&mut file) {
                Ok(record) if record.seq >= req.start_seq => {
                    stream.write_all(&record.bytes).await?;
                }
                Ok(_) => continue,  // Skip records before start_seq
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    // Reached end of file, wait for more
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}
```

Consumer connects and reads:

```rust
// rsx-dxs/src/client.rs (simplified)
pub struct DxsConsumer {
    stream_id: u32,
    producer_addr: String,
    tip: u64,  // Last seq processed
}

impl DxsConsumer {
    pub async fn poll(&mut self) -> io::Result<Option<RawWalRecord>> {
        // Connect if needed
        if self.stream.is_none() {
            let stream = TcpStream::connect(&self.producer_addr).await?;
            let req = ReplayRequest {
                stream_id: self.stream_id,
                start_seq: self.tip + 1,
            };
            stream.write_all(as_bytes(&req)).await?;
            self.stream = Some(stream);
        }

        // Read next record
        let stream = self.stream.as_mut().unwrap();
        let mut header_buf = [0u8; 16];
        stream.read_exact(&mut header_buf).await?;
        let header = parse_header(&header_buf)?;

        let mut payload = vec![0u8; header.payload_len as usize];
        stream.read_exact(&mut payload).await?;

        // Validate CRC
        let crc = compute_crc32(&payload);
        if crc != header.crc32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "crc mismatch",
            ));
        }

        self.tip = header.seq;
        Ok(Some(RawWalRecord { header, payload }))
    }
}
```

## How It Works in RSX

Matching engine (producer):
1. Writes fills to WAL (16B header + 64B payload)
2. Flushes every 10ms or 1000 records
3. Runs DxsReplay server on TCP port (e.g., 9001)

Risk engine (consumer):
1. Connects to ME's DxsReplay server
2. Sends `ReplayRequest { stream_id: 1, start_seq: 1234 }`
3. Reads records as they're produced
4. Processes fills, updates positions
5. Periodically persists tip (last seq processed)

Marketdata (consumer):
1. Connects to ME's DxsReplay server
2. Sends `ReplayRequest { stream_id: 1, start_seq: 1 }`
3. Reads fills + BBO updates
4. Builds shadow orderbook
5. Publishes L2/BBO to public WebSocket

Recorder (consumer):
1. Connects to ME's DxsReplay server
2. Reads all records from seq=1
3. Writes to daily archive files
4. Infinite retention (compliance requirement)

## No Kafka Means

**No offset management**: Consumer persists its own tip. ME doesn't
track "who read what." If consumer crashes, it rereads from tip+1.

**No partitioning**: One stream per symbol. Bitcoin = stream_id 1.
Ethereum = stream_id 2. Consumers subscribe to the streams they care
about.

**No rebalancing**: Consumer connects directly to producer. If producer
dies, consumer waits. When producer restarts, consumer reconnects and
replays from tip+1.

**No schema registry**: Payload is raw bytes. Consumer casts based on
`header.record_type`. Unknown types = skip.

**No lag monitoring**: Consumer tracks its own lag (`current_seq -
tip`). Producer doesn't know.

## Recovery Semantics

Producer crashes:
1. Restart producer
2. Load snapshot (if exists)
3. Replay WAL from snapshot_seq+1
4. Resume at current seq
5. Consumers reconnect, resume from their tip+1

Consumer crashes:
1. Restart consumer
2. Load tip from disk
3. Connect to producer
4. Send `ReplayRequest { start_seq: tip+1 }`
5. Replay all records since tip

Both crash (dual failure):
1. Producer restarts first, loads state
2. Consumer restarts, connects, replays from tip+1
3. Live sync achieved within 100ms (bounded lag)

## Tests Prove It

```rust
// rsx-dxs/tests/tls_test.rs
#[tokio::test]
async fn consumer_replays_from_tip() {
    let tmp = TempDir::new().unwrap();

    // Producer writes 100 records
    let mut writer = WalWriter::new(1, tmp.path(), None, 64*1024*1024, 600_000_000_000).unwrap();
    for i in 0..100 {
        writer.append(&mut make_fill(i)).unwrap();
    }
    writer.flush().unwrap();

    // Start replay server
    let replay = DxsReplay::new(1, tmp.path().to_path_buf()).await.unwrap();
    tokio::spawn(async move { replay.run().await });

    // Consumer reads from seq 50
    let tip_file = tmp.path().join("tip.txt");
    std::fs::write(&tip_file, "50").unwrap();

    let mut consumer = DxsConsumer::new(1, "127.0.0.1:9001".to_string(), tip_file, None).unwrap();

    let first_record = consumer.poll().await.unwrap().unwrap();
    assert_eq!(first_record.header.seq, 51);  // tip+1
}
```

Gap detection:

```rust
#[tokio::test]
async fn consumer_detects_sequence_gap() {
    let mut consumer = DxsConsumer::new(/* ... */);

    let rec1 = consumer.poll().await.unwrap().unwrap();
    let rec2 = consumer.poll().await.unwrap().unwrap();

    if rec2.header.seq != rec1.header.seq + 1 {
        panic!("gap detected: {} -> {}", rec1.header.seq, rec2.header.seq);
    }
}
```

## Why It Matters

Kafka cluster: 3 brokers, 3 ZooKeeper nodes, ops team, 6 VMs.

DXS: TCP listener on existing process, 50 lines of code.

Latency:
- Kafka: 5-10ms producer → consumer
- DXS: 10-100μs producer → consumer (same machine), 1-5ms (cross-rack)

Complexity:
- Kafka: partition assignment, rebalancing, offset commits, consumer
  groups, schema registry
- DXS: connect, read, persist tip

Failure modes:
- Kafka: broker down, ZK split-brain, rebalance storm, offset loss
- DXS: producer down (consumers wait), consumer down (reconnect), TCP
  disconnect (reconnect)

## The Trade-off

DXS is not a general-purpose message broker:
- No fan-out optimization (each consumer reads WAL independently)
- No retention policy (producer decides when to rotate/delete)
- No exactly-once delivery (consumer must deduplicate)

DXS is perfect for:
- Event sourcing (single producer, multiple consumers)
- Audit logs (immutable append-only stream)
- State replication (matching engine → risk engine)

**We don't need Kafka's features. We just need TCP + file seek.**

## Key Takeaways

- **Producer = broker**: Serve your own WAL over TCP, skip the middleman
- **Consumer owns tip**: No offset manager, just persist last seq
  processed
- **Replay = seek + stream**: WAL file is the message log
- **10-100μs latency**: No broker hop, direct producer → consumer
- **50 lines of code**: TCP listener, not a distributed system

Every producer in RSX is a DxsReplay server. Every consumer is a
DxsConsumer client. No Kafka cluster. No ops burden.

When the matching engine produces a fill, consumers see it 10μs later.
Not 10ms. Not "eventually." 10 microseconds.

## Target Audience

Developers considering Kafka/NATS/Pulsar for event streaming. Anyone
building low-latency replication. SREs tired of operating Kafka
clusters. Engineers who've read the Kafka docs and thought "I just need
TCP."

## See Also

- `specs/1/10-dxs.md` - DXS protocol specification
- `specs/1/48-wal.md` - WAL format (same as DXS stream format)
- `rsx-dxs/src/server.rs` - DxsReplay server implementation
- `rsx-dxs/src/client.rs` - DxsConsumer client implementation
- `rsx-dxs/tests/tls_test.rs` - End-to-end DXS tests
- `blog/12-deleted-serialization.md` - Why disk = wire = memory format
