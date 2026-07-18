# demo/rsx-playground — the dashboard walkthrough

The rendered demo of the playground dashboard: an **interactive operator's
tour** filmed against the live demo scene (`../demo_scenario.py`: calm →
+45% PENGU pump with a real risk-engine liquidation → retrace), stitched
to a GIF for `../README.md`. Inherits the repo-root `../../CLAUDE.md` and
the crate-local `../CLAUDE.md` (Ayam Cemani visual system).

Unlike the old static page-flip, every step shows the dashboard DOING
something against real action: an order landing, the book marching up
mid-pump, a crash healing, the invariants holding after the liquidation.
Nothing is faked — the fills, the liquidation, and the latency numbers
are the live system's own.

## What the tour shows (`dashboard-walkthrough.sh`)

1. **Overview + Control** (calm) — the managed cluster green/live and the
   start/stop surface. `mark` shows stopped BY SCENE DESIGN: the scene
   silences it so risk marks PENGU to the local BBO index (see
   `demo_scenario.py prep`), making the scripted book move the margin price.
2. **Orders** — a quick BUY 20 clicked on the page; the next frame shows
   it landing (fill result + recent-orders row).
3. **Book × 3** (during the pump) — three beats of PENGU's ladder with the
   mid marching up; short frame durations so the march reads as motion.
4. **Risk** (after `LIQUIDATION confirmed` in the scene log) — positions
   after the risk engine force-closed the victim's leveraged short.
5. **Recovery** — SIGKILL the recorder from the fault-inject pane, then
   the live feed showing the watcher heal it (red → green, new pid).
6. **WAL + Cast** — the tape: WAL streams/tips and casting flows.
7. **Latency** — one real e2e probe fired. The page's own caption is the
   honest one: this is the playground ↔ gateway demo path (~ms), NOT the
   exchange's µs internals — Criterion benches + `make perf-gate` own
   those (root CLAUDE.md "Latency & performance verification").
8. **Verify** — the 10 invariants + reconciliation, run AFTER the
   liquidation: "Position = sum of fills" PASS is the accounting payoff.
   The one FAIL ("6/7 running; down: mark") and the overview's RED health
   bar are the stopped mark process from (1) — scene design, not a defect.
   Leave them in; an all-green frame would require faking the scene.

## How it's rendered

`dashboard-walkthrough.sh` preps the scene, starts `demo-scene run` in the
background, then drives agent-browser (viewport 1280×800), synchronizing
shots to the scene's stdout markers (`PUMP begins`, `LIQUIDATION
confirmed`). PIL stitches the shots (960×600, adaptive 128-color palette,
per-frame durations), `gifsicle -O3 --lossy` shrinks the result.

    ./rsx-playground/demo/dashboard-walkthrough.sh            # prep + record
    SKIP_PREP=1 ./rsx-playground/demo/dashboard-walkthrough.sh # cluster already prepped

## Fresh-scene runbook

- The script's own `prep` stage boots the right cluster (fresh PENGU-only
  minimal scenario, victim short staged) — several minutes. Don't record
  against a stale cluster: the order/event path wedges minutes after boot
  (BUGS.md `RISK-CMP-DESYNC-ORDER-EVENT-STARVE`) and the dashboard's book
  display can starve (BUGS.md `DEMO-MD-DISPLAY-STARVE`). Prep, then
  capture immediately; if the book frames don't show the mid moving,
  reboot the scene and re-record — never present a frozen book as live.
- The scene drifts forever after the retrace; the script kills `run` and
  `demo-scene clean`s on exit, leaving the cluster up.
- Frame checkers: open the GIF frames (PIL) and confirm the order landed
  (fill in `#quick-result` + recent-orders row), the three book frames
  show different mids, the recovery feed shows the recorder restart, and
  verify shows all invariants green.

## Artifacts

- `dashboard-walkthrough.sh` — the driver (checked in; reproducible).
- `dashboard.gif` — the rendered tour (embedded in `../README.md`).
