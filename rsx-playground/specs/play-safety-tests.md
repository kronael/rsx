# Playwright: Safety, Crash & Handover Tests

## Goal

Comprehensive Playwright tests covering process crash
recovery, graceful shutdown, session handover, and
operational safety edge cases.

## File

`rsx-playground/tests/play_safety.spec.ts` (new file)

## Tests (~25 tests)

### Process Crash & Recovery (8 tests)

1. **gateway crash shows error state in topology**
   - POST `/api/processes/gw-0/stop`
   - Verify topology node shows red dot within 5s
   - Verify orders page shows "gateway not running"

2. **gateway restart recovers order flow**
   - Stop gateway, verify error state
   - POST `/api/processes/gw-0/start`
   - Verify topology dot turns green within 10s
   - Submit test order, verify accepted/simulated

3. **maker crash shows stopped in maker tab**
   - POST `/api/processes/maker/stop`
   - Verify maker status shows `running: false`
   - Verify book still shows seeded levels (sim)

4. **maker restart resumes quoting**
   - Stop then start maker
   - Poll `/api/maker/status` until running=true
   - Verify book updates within 10s

5. **all-stop clears topology to red**
   - POST `/api/processes/all/stop?confirm=yes`
   - Verify all topology nodes show red dots
   - Verify pulse bar shows 0 processes

6. **all-start recovers from all-stop**
   - Stop all, then start all
   - Poll until gateway running
   - Verify >=4 processes in topology

7. **rapid stop/start doesn't corrupt state**
   - Stop gateway, immediately start gateway
   - Verify no duplicate process entries
   - Verify order submission works after settle

8. **process crash preserves session**
   - Note session_id from `/api/sessions/status`
   - Stop/start gateway
   - Verify session_id unchanged

### Session Safety (5 tests)

9. **session collision returns 409**
   - Allocate session A
   - Try allocate session B → expect 409
   - Release A

10. **stale session auto-releases after lease TTL**
    - Allocate session, don't renew
    - Wait for lease expiry (mock or fast-forward)
    - New allocate succeeds

11. **session renew extends TTL**
    - Allocate session
    - Renew session → verify ttl_remaining increases
    - Verify session still active after original TTL

12. **release then allocate works immediately**
    - Allocate, release, allocate again → expect 200

13. **invalid session_id renew returns 409**
    - Allocate session
    - Try renew with wrong session_id → 409

### Operational Safety (6 tests)

14. **confirm=yes required for destructive actions**
    - POST `/api/processes/all/stop` without confirm
    - Expect rejection / confirmation prompt
    - POST with `confirm=yes` succeeds

15. **run_id mismatch blocks process control**
    - Allocate session (get run_id)
    - POST start with wrong X-Run-Id → 409

16. **audit log records all actions**
    - Submit order, stop process, start process
    - GET `/api/audit-log`
    - Verify all 3 actions logged with timestamps

17. **concurrent order submissions don't crash**
    - Fire 10 parallel POST `/api/orders/test`
    - All return 200 (not 500)
    - recent_orders has all 10

18. **idempotency key prevents duplicate orders**
    - POST order with X-Idempotency-Key: "test-1"
    - POST same key again → "duplicate submission"

19. **invalid form data returns error, not 500**
    - POST order with price="" → error message
    - POST order with qty="abc" → error message
    - POST order with symbol_id="-1" → error message

### Graceful Degradation (6 tests)

20. **book page works with no processes**
    - Stop all processes
    - Navigate to /book → shows seeded sim data
    - No console errors

21. **risk page works with no postgres**
    - Navigate to /risk
    - Lookup user → "not connected" message
    - All cards still render (no 500s)

22. **WAL page works with no WAL files**
    - Navigate to /wal
    - Shows "no WAL events" or sim events
    - Filter radios still functional

23. **orders page works with gateway down**
    - Stop gateway
    - Submit order → "simulated (gateway offline)"
    - Recent orders table updates

24. **topology works with all processes stopped**
    - Stop all
    - Navigate to /topology
    - All nodes show red dots, no JS errors

25. **overview pulse bar handles zero state**
    - Stop all processes
    - Pulse bar shows 0 processes, 0 ord/s
    - No console errors or 500s

## Acceptance Criteria

- [ ] All 25 tests pass with exchange offline
- [ ] Tests that stop/start processes clean up after
- [ ] No test depends on previous test state
- [ ] Each test <15s timeout
- [ ] File follows existing patterns from play_*.spec.ts

## Constraints

- Use `test.describe.serial` for stop/start sequences
- Import helpers from `test_helpers.ts`
- Use `request` fixture for API calls (not page.request)
- Clean up: restart gateway at end of each stop test
- Tag: `[safety]` in test.describe
