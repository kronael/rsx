# pq — Parquet query tool

Source: [github.com/tonivade/pq](https://github.com/tonivade/pq).

`jq`-equivalent for Parquet files. Read, filter, convert `.parquet` from the CLI.

```bash
pq read file.parquet                              # → JSON
pq read --format csv file.parquet                 # → CSV
pq read --filter 'type == "FILL"' file.parquet    # filter rows
pq read --select seq,price,qty file.parquet       # column projection
pq schema file.parquet                            # inspect schema
```

Relevant for RSX when WAL records are archived as Parquet for backtesting or ML
training. Pair with `rsx-cli dump`:

```bash
rsx-cli dump stream_1/00001.wal | pq write --schema schema.txt output.parquet
pq read --filter 'type == "FILL"' output.parquet
```

Keep as an external tool — avoids pulling `arrow`/`parquet` dependencies into `rsx-cli`.
