# DONE: Bugs Fixed (11)

Fixed 2026-02-14 in playground CLI and server.py.

## server.py (10 fixes applied)

### [x] Missing 3rd return value in send_order_to_gateway
- All error paths now return (None, error_msg, None) tuple.

### [x] Missing await after kill() in lifespan cleanup
- Added `await info["proc"].wait()` after kill.

### [x] XSS in stress reports list
- Added `import html`, escape ts and id before rendering.

### [x] PID file written before stability check
- Write PID immediately, clean up if process exits.

### [x] Managed dict not cleaned in stop_process
- Added `del managed[name]` after successful stop.

### [x] Scenario switch missing stop_all()
- Added `await stop_all()` and sleep before restart.

### [x] Scenario state set before success
- Moved `current_scenario = scenario` inside success branch.

### [x] Port kill wait time too short
- Changed from 0.5s to 2.0s for OS port release.

### [x] Lifespan cleanup doesn't clear managed dict
- Added `managed.clear()` after process cleanup loop.

### [x] Health check returns static info
- Now returns process count, postgres status.

## playground CLI (10 fixes applied)

### [x] ensure_server() doesn't propagate failure
- Returns True/False, callers check return value.

### [x] api_post() status validation
- Returns True/False instead of response object.
- All 6 callers updated to check truthiness.

### [x] TOCTOU race on PID file
- Added FileNotFoundError handler.

### [x] Integer conversion without error handling
- Added try/except ValueError for stress test args.

### [x] PermissionError doesn't clean PID file
- Now tries to unlink stale PID, returns False.

### [x] ensure_server() silent failure
- Returns False when health check loop exhausts.

### [x] restart missing stop check
- Checks stop_server() return before starting.

### [x] Socket double-close
- Removed premature sock.close() before return.

### [x] logs_cmd() ignores subprocess return code
- Now returns result.returncode and 130 for SIGINT.

### [x] send_order_to_gateway all error paths fixed
- All 3 except branches now return 3-tuple with None.
