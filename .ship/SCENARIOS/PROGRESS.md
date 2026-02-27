# PROGRESS

updated: Feb 27 18:29:24  
phase: executing

```
[░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░] 0%  0/3
```

| | count |
|---|---|
| completed | 0 |
| running | 3 |
| pending | 0 |
| failed | 0 |

## workers

- w0: Fix rsx-risk multi-symbol routing. Read `RSX_ME_CMP_ADDRS` (comma-separated SocketAddrs) from env at startup, falling back to `RSX_ME_CMP_ADDR` if the multi-addr var is absent. Build a `HashMap&lt;u32, SocketAddr&gt;` keyed by symbol_id parsed from the address port (port - 9100 = symbol_id). Route all outbound CMP order messages to the correct ME socket by looking up the order's symbol_id. Apply the same fix to Risk replica startup. Verify with `cargo check -p rsx-risk` and existing tests passing.
- w1: Fix rsx-marketdata multi-symbol subscriptions. Read `RSX_ME_CMP_ADDRS` (comma-separated) from env at startup, falling back to `RSX_ME_CMP_ADDR`. On startup, open a CMP subscription to every address in the list so the single marketdata process receives fills/updates from all MEs. The existing single WS port (8180) and `S:[sym, channels]` subscription protocol are unchanged — clients already filter by symbol. Verify with `cargo check -p rsx-marketdata` and existing tests passing.
- w2: Fix the `./start` Python script: (1) in `build_spawn_plan()`, compute `me_cmp_addrs` as a comma-joined string of all ME CMP addresses (9100 + symbol_id for each symbol) and pass it as `RSX_ME_CMP_ADDRS` to Risk, Risk replica, and Marketdata env dicts; (2) build the Mark Binance combined-stream URL from all symbols in config (e.g. `pengusdt@trade/solusdt@trade`) instead of `symbols[0]`; (3) append one `("recorder", "target/debug/rsx-recorder", env)` entry per scenario with `RSX_RECORDER_STREAM_ID` set to the first symbol's id, `RSX_RECORDER_PRODUCER_ADDR` to the first ME's replay address, `RSX_RECORDER_ARCHIVE_DIR=./tmp/wal/archive`, and `RSX_RECORDER_TIP_FILE=./tmp/recorder-tip-&lt;sid&gt;`. Acceptance: `./start minimal` spawn plan has 6 entries, `duo` has 7, `full` has 9, `stress` has 12.
