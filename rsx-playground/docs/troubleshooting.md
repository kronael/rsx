# Troubleshooting

Common issues and fixes for the RSX playground.

## Processes won't start

Check that binaries are built:

```
cargo build
```

Then restart from the Control page or:

```
cd rsx-playground && uv run server.py
```

## No orderbook data

The matching engine needs at least one order.
Use the Orders tab to place a limit order,
or start the market maker from the Control page.

## WAL lag increasing

The WAL writer flushes every 10ms.
If lag exceeds 100ms, check disk I/O:

```
iostat -x 1
```

## Port conflicts

Default port is 49171. Override with:

```
PORT=8080 uv run server.py
```

## Market maker not quoting

Check the maker status at `/api/maker/status`.
If level count is 0, check `tmp/maker-status.json`.
The maker requires the matching engine to be running.

## Playwright tests failing

Run the full suite:

```
cd rsx-playground && bunx playwright test
```

For headless debug:

```
bunx playwright test --headed
```
