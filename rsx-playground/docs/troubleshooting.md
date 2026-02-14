# Playground Troubleshooting

Common issues and solutions when using the RSX Playground.

## Playground Won't Start

### Port 49171 already in use

**Error:**
```
OSError: [Errno 98] Address already in use
```

**Solution:**
```bash
# Find and kill the existing process
lsof -i :49171
kill <PID>

# Or use a different port
PORT=50000 uv run server.py
```

### Python/uv not found

**Error:**
```
command not found: uv
```

**Solution:**
```bash
# Install uv
curl -LsSf https://astral.sh/uv/install.sh | sh

# Or use pip
pip install -r requirements.txt
python server.py
```

## Processes Won't Start

### Build & Start All does nothing

**Symptoms:**
- Button clicked, no processes appear
- No error message

**Check:**
1. Open browser console (F12), look for JavaScript errors
2. Check playground logs (terminal where `uv run server.py` runs)
3. Click Overview tab, wait 5s for auto-refresh

**Solution:**
```bash
# Check if Cargo is in PATH
cargo --version

# If not, add to PATH
export PATH="$HOME/.cargo/bin:$PATH"
```

### Processes start then immediately stop

**Symptoms:**
- Process appears in table, then disappears
- PID shown briefly

**Check Logs tab** for errors. Common causes:

1. **Port already in use**
   ```
   error: Address already in use (os error 98)
   ```
   Solution: Stop conflicting process or change ports in `server.py`

2. **Postgres not running**
   ```
   error: Connection refused (os error 111)
   ```
   Solution: Start Postgres or use minimal scenario (no DB)

3. **WAL directory permission denied**
   ```
   error: Permission denied (os error 13)
   ```
   Solution:
   ```bash
   chmod -R 755 ./tmp
   rm -rf ./tmp/*
   ```

### Build hangs at "building..."

**Symptoms:**
- Spinner spins forever
- No error, no processes start

**Check:**
```bash
# In another terminal
ps aux | grep cargo

# If stuck, kill
killall cargo
```

**Solution:**
```bash
# Build manually to see error
cd rsx-gateway
cargo build
```

### "No such file or directory" when starting process

**Error in logs:**
```
FileNotFoundError: [Errno 2] No such file or directory: '../target/debug/rsx-gateway'
```

**Solution:**
```bash
# Build all binaries
cargo build --workspace

# Or from playground directory
cd ..
cargo build --workspace
cd rsx-playground
```

## Orders Not Submitting

### "WebSocket connection failed"

**Symptoms:**
- Order submission returns error
- Recent orders empty

**Check:**
1. Gateway process running (Overview tab)
2. Gateway port 8080 accessible:
   ```bash
   curl http://localhost:8080/health
   ```

**Solution:**
```bash
# Restart Gateway
curl -X POST http://localhost:49171/api/processes/gateway/restart
```

### Orders submitted but no fills

**Symptoms:**
- Order shows "submitted" status
- Never changes to "filled" or "done"

**Check:**
1. ME process running (Overview tab)
2. Risk process running
3. WAL lag (WAL tab) - should be <100ms
4. Logs tab for errors

**Possible causes:**
- ME crashed (check Logs tab)
- Risk rejected order (check Logs tab for "margin check failed")
- WAL replication stalled (check WAL tab for lag)

**Solution:**
```bash
# Restart entire pipeline
curl -X POST http://localhost:49171/api/processes/all/stop
curl -X POST http://localhost:49171/api/processes/all/start
```

### "Order rejected: insufficient margin"

**Symptoms:**
- Order immediately rejected
- Status shows "failed"

**Check Risk tab:**
1. Lookup user_id
2. Check available balance
3. Check frozen margin

**Solution:**
```bash
# Deposit more collateral
curl -X POST http://localhost:49171/api/users/1/deposit \
  -H "Content-Type: application/json" \
  -d '{"amount": 10000}'

# Or use smaller order size
```

## UI Issues

### Tabs not loading / showing "loading..."

**Symptoms:**
- Card content stuck on "loading..."
- Auto-refresh not working

**Check:**
1. Browser console (F12) for HTMX errors
2. Network tab for failed requests
3. Playground terminal for Python exceptions

**Solution:**
```bash
# Hard refresh browser
Ctrl+Shift+R  # Linux/Windows
Cmd+Shift+R   # Mac

# Or clear browser cache
```

### "No data" everywhere

**Symptoms:**
- All tabs show "no data" or empty tables
- Processes show as running

**Check:**
1. Are processes actually running?
   ```bash
   ps aux | grep rsx
   ```
2. Are WAL files being written?
   ```bash
   ls -lh tmp/wal/
   ```

**Solution:**
- Submit test orders (Orders tab)
- Wait 5-10s for auto-refresh

### Logs tab not showing recent logs

**Symptoms:**
- Logs tab empty or old logs only
- Processes running and logging to terminal

**Cause:** Playground reads from `./tmp/unified.log`, but processes may be logging elsewhere.

**Solution:**
```bash
# Check where processes are logging
ps aux | grep rsx | grep -v grep

# If logging to stdout/stderr, redirect:
# (Modify server.py to add stdout/stderr redirect)
```

## Performance Issues

### Playground UI slow / laggy

**Symptoms:**
- Auto-refresh takes >5s
- Clicking tabs lags

**Cause:** Too many processes or large log files

**Solution:**
```bash
# Stop stress scenarios
curl -X POST http://localhost:49171/api/processes/all/stop

# Clear logs
rm -f ./tmp/unified.log

# Restart playground
# (Ctrl+C, then uv run server.py)
```

### High CPU usage on playground process

**Symptoms:**
- `python server.py` using >50% CPU

**Cause:** Parsing large log files on every request

**Solution:**
```bash
# Truncate logs
: > ./tmp/unified.log

# Or disable auto-refresh on Logs tab
# (Click away from Logs tab)
```

## WAL Issues

### "WAL files not found"

**Symptoms:**
- WAL tab shows no files
- Processes running

**Check:**
```bash
ls -lh tmp/wal/
```

**Solution:**
```bash
# Create WAL directories
mkdir -p tmp/wal
chmod 755 tmp/wal

# Restart processes
curl -X POST http://localhost:49171/api/processes/all/restart
```

### WAL lag increasing

**Symptoms:**
- Lag dashboard shows growing lag (>1s)
- Orders slowing down

**Check:**
1. Disk space: `df -h`
2. Disk I/O: `iostat -x 1`
3. Consumer process running (Recorder)

**Solution:**
```bash
# Restart consumer
curl -X POST http://localhost:49171/api/processes/recorder/restart

# Or stop load generator
curl -X POST http://localhost:49171/api/processes/all/stop
```

## Database Issues

### "Connection refused" (Postgres)

**Symptoms:**
- Gateway/Risk won't start
- Error: `Connection refused (os error 111)`

**Solution:**
```bash
# Start Postgres
sudo systemctl start postgresql
# Or
docker run -d -p 5432:5432 -e POSTGRES_PASSWORD=test postgres

# Or use minimal scenario (no DB required)
```

### "Schema mismatch"

**Symptoms:**
- Gateway starts then crashes
- Error: `table "users" not found`

**Solution:**
```bash
# Reset database
./scripts/reset-db.sh

# Or from playground
curl -X POST http://localhost:49171/api/db/reset
```

## Recovery Procedures

### Clean restart

```bash
# Stop all processes
curl -X POST http://localhost:49171/api/processes/all/stop

# Clean WAL and logs
rm -rf tmp/wal/* tmp/*.log

# Restart
curl -X POST http://localhost:49171/api/processes/all/start
```

### Full reset

```bash
# Stop playground
# Ctrl+C in terminal

# Clean everything
cd rsx-playground
rm -rf tmp/*
rm -rf .pytest_cache

# Rebuild
cd ..
cargo clean
cargo build --workspace

# Restart playground
cd rsx-playground
uv run server.py
```

### Check playground logs

```bash
# Playground logs to stdout, redirect to file:
uv run server.py 2>&1 | tee playground.log
```

## Known Issues

### Playwright tests fail intermittently

**Symptom:** E2E tests pass sometimes, fail other times

**Cause:** Auto-refresh timing, network delays

**Workaround:**
```bash
# Increase timeout in tests
# Or run with --max-failures=1 --reruns=2
pytest tests/ --max-failures=1 --reruns=2
```

### Browser shows stale data after restart

**Symptom:** Tab shows old process states after restart

**Cause:** Browser cached HTMX responses

**Solution:** Hard refresh (`Ctrl+Shift+R`)

### "Too many open files"

**Symptom:** Processes fail to start after many restarts

**Cause:** File descriptor limit

**Solution:**
```bash
# Increase limit
ulimit -n 4096

# Or restart playground
```

## Getting Help

If none of the above solutions work:

1. Check [PROGRESS.md](../PROGRESS.md) for known issues
2. Check [CLAUDE.md](../CLAUDE.md) for development notes
3. Read full architecture: [specs/v1/ARCHITECTURE.md](../specs/v1/ARCHITECTURE.md)
4. Check Logs tab for specific error messages
5. Run manual tests:
   ```bash
   cd ..
   cargo test --workspace
   ```
