# Recovery Explorer — playground plan

Goal (user): recovery from faults is **explored on the playground itself**,
precisely, with the recovery **events flowing in live** — not documented
elsewhere. A visitor injects a fault and *watches the system heal*.

## Three fault classes + how each is explored

1. **Process kill → observe recovery on Overview.**
   - Inject: kill a managed process (existing `stop-proc` / an explicit
     "kill gw-0" button). The `_process_watcher` (server.py:690) already
     auto-restarts with bounded backoff.
   - Observe: the **Overview** screen shows the process flip red →
     restarting → green, the restart count + backoff window, the pid change.
     Recovery timeline: killed_at → respawn_at → healthy_at.

2. **Network faults → `tc` / `iptables` directly.**
   - Inject: `tc qdisc add dev lo root netem delay 50ms loss 10%`, or an
     `iptables` DROP on a cluster port (partition risk↔ME). A "network
     fault" control shells these (sudo).
   - Observe: casting `FAULTED` → NAK/replay recovery; BBO/book stalls then
     re-syncs. Clear the qdisc/rule to heal; watch catch-up.

3. **WAL corruption → manual hex edit.**
   - Inject: flip a byte in a WAL record (a "corrupt WAL" control that seeks
     a record's CRC and XORs a byte), OR document the `xxd`/hex-edit steps
     for the visitor to do by hand.
   - Observe: on replay the CRC32C check fails → the recovery path (skip the
     torn tail / halt-and-report). Show which record, which seq.

## The live event feed (the "events flow in" part)

A **Recovery** tab (or extend Faults), two panes:
- **left — inject:** the three controls above, each with a confirm gate.
- **right — live recovery feed:** auto-refreshing (HTMX poll ~500ms or SSE)
  stream of recovery events: process state changes (`scan_processes`),
  `FAULTED`/`replay`/`caught-up` (parsed from the risk/ME/marketdata logs —
  the `WARN rsx_*::failover` + `cmp receiver FAULTED` lines already exist),
  WAL `CRC error` lines. Each event stamped, newest-first, colored by class
  (red fault → amber recovering → green healed).

## Implementation notes (execute after the publish thrust)

- Reuse `scan_processes()` for state; `read_logs(process=…)` for the
  FAULTED/replay/CRC lines (the `risk.gw FAULTED … opening replay` /
  `cmp receiver FAULTED` lines are already in log/*.log).
- New endpoints: `POST /api/fault/kill`, `POST /api/fault/net` (tc/iptables,
  sudo), `POST /api/fault/wal-corrupt`; `GET /x/recovery-feed` (live partial).
- Page: `pages.py` `recovery_page()` + partials; add "Recovery" to nav
  (update the tab-count nav test).
- Style per rsx-playground/CLAUDE.md (surrounding-ring callouts, semantic
  colors). Confirm gates on all inject actions.
- Tests: `play_recovery.spec.ts` — kill a proc, assert the feed shows the
  restart + Overview goes green; net-fault + heal; WAL-corrupt shows a CRC
  event. Gate live-cluster tests like the e2e_* ones.

## Open question for the founder

Network-fault + WAL-corrupt run `sudo tc`/`iptables` and write to WAL files —
genuinely destructive on a shared box. Gate behind `RSX_PLAYGROUND_UNSAFE=1`
so the public demo can't brick the cluster; expose only the read-only
"kill + observe" flow by default?
