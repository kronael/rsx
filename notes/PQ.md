# pq - Parquet Query Tool

**URL:** https://github.com/tonivade/pq

## Overview

pq is a jq-equivalent for Parquet files, providing command-line querying and conversion capabilities.

## Key Features

### 1. Bidirectional Conversion
- **JSON → Parquet:** `pq write --schema=schema.txt output.parquet < input.jsonl`
- **Parquet → JSON:** `pq read example.parquet` (outputs JSON)
- **Parquet → CSV:** `pq read --format csv example.parquet`

### 2. Query Operations
- **Filtering:** `pq read --filter 'gender == "Male"' example.parquet`
- **Column selection:** `pq read --select col1,col2 example.parquet`
- **Pagination:** `--head N`, `--tail N`, `--skip N`
- **Row counting:** `pq count --filter 'age > 30' example.parquet`

### 3. Schema Inspection
- **View schema:** `pq schema example.parquet`
- **Schema filtering:** Filter schema by column names

### 4. Metadata
- **File metadata:** `pq metadata example.parquet`

## Installation

Download precompiled binaries (Linux/macOS/Windows) from GitHub releases.
Built with GraalVM CE 25.0.0 for native performance.

**Current version:** v0.8.0

## Use Cases for RSX

### WAL Analysis Workflow

1. **Dump WAL to JSON:**
   ```bash
   rsx-cli dump stream_1/00001.wal > records.jsonl
   ```

2. **Convert to Parquet (optional):**
   ```bash
   pq write --schema schema.txt records.parquet < records.jsonl
   ```

3. **Query with pq:**
   ```bash
   # Filter by record type
   pq read --filter 'type == "FILL"' records.parquet

   # Count specific records
   pq count --filter 'seq > 1000' records.parquet

   # Select specific columns
   pq read --select seq,type,len records.parquet
   ```

4. **Or query JSON directly with jq:**
   ```bash
   rsx-cli dump file.wal | jq 'select(.type == "FILL")'
   ```

## Recommendation for RSX

**Use pq as external tool, not embedded:**
- rsx-cli outputs JSON (simple, universal)
- Users install pq separately if they want Parquet
- Avoids heavy arrow/parquet dependencies in rsx-cli
- pq is more featureful than we could build

## Schema Example for WAL Records

```
seq: INT64
type: STRING
len: UINT32
crc32: STRING
```

Save to `wal-schema.txt`, then:
```bash
rsx-cli dump file.wal | pq write --schema wal-schema.txt output.parquet
```
