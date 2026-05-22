/**
 * Deterministic single-worker validation pipeline.
 *
 * Runs sequentially (workers: 1 enforced globally) after infra-smoke.
 * All product shards that need a live exchange (process-control,
 * market-maker, trade-ui) depend on this project so they are skipped
 * immediately when the system is not ready — rather than failing
 * mid-test with obscure errors.
 *
 * Each test is a discrete readiness gate; order is deterministic:
 *   1. core processes running
 *   2. gateway reachable
 *   3. maker running + pid present
 *   4. book seeded (best_bid > 0 && best_ask > 0)
 *   5. /api/status embeds "maker" key
 */

import { test, expect } from "@playwright/test";

const SYMBOL_ID = 10;
const BASE = "http://localhost:49171";

test.describe("Readiness pipeline", () => {
  // Auto-heal: if an upstream shard (e.g. safety) left the
  // topology degraded, readiness should self-restore rather
  // than cascade-fail the 200+ downstream shards. Cheap when
  // already-running (server responds "already running").
  test.beforeAll(async () => {
    try {
      const r = await fetch(`${BASE}/api/processes`);
      if (r.ok) {
        const procs: Array<{ name: string; state: string }> =
          await r.json();
        const running = procs.filter((p) => p.state === "running");
        const gatewayUp = running.some(
          (p) => p.name.includes("gateway") || p.name.startsWith("gw"),
        );
        if (gatewayUp && running.length >= 4) return;
      }
    } catch {
      // fall through to restore
    }
    await fetch(
      `${BASE}/api/processes/all/start?scenario=minimal&confirm=yes`,
      { method: "POST" }
    );
    await new Promise((r) => setTimeout(r, 5000));
  });

  // ── 1. Core processes ──────────────────────────────────────

  test("core processes: >=4 running including gateway",
    async ({ request }) => {
      const r = await request.get("/api/processes");
      expect(r.ok()).toBe(true);
      const procs: Array<{ name: string; state: string }> =
        await r.json();
      const running = procs.filter((p) => p.state === "running");
      const gatewayUp = running.some(
        (p) => p.name.includes("gateway") || p.name.startsWith("gw"),
      );
      expect(gatewayUp, "gateway process not running").toBe(true);
      expect(running.length).toBeGreaterThanOrEqual(4);
    },
  );

  // ── 2. Gateway reachable ───────────────────────────────────

  test("gateway: /api/status responds 200", async ({ request }) => {
    const r = await request.get("/api/status");
    expect(r.ok()).toBe(true);
    const body = await r.json();
    // status object has at least one key
    expect(Object.keys(body).length).toBeGreaterThan(0);
  });

  // ── 3. Maker running ───────────────────────────────────────

  test("maker: running=true and pid>0", async ({ request }) => {
    const r = await request.get("/api/maker/status");
    expect(r.ok()).toBe(true);
    const status = await r.json();
    expect(status.running, "maker not running (exchange offline)").toBe(true);
    expect(typeof status.pid).toBe("number");
    expect(status.pid).toBeGreaterThan(0);
  });

  // ── 4. Book seeded ─────────────────────────────────────────

  test("book: best_bid > 0 and best_ask > 0", async ({ request }) => {
    const r = await request.get(`/api/book/${SYMBOL_ID}`);
    expect(r.ok()).toBe(true);
    const book = await r.json();
    expect(
      book.bids?.length ?? 0,
      "no bids in book",
    ).toBeGreaterThanOrEqual(1);
    expect(
      book.asks?.length ?? 0,
      "no asks in book",
    ).toBeGreaterThanOrEqual(1);
    expect(book.bids[0].px).toBeGreaterThan(0);
    expect(book.asks[0].px).toBeGreaterThan(0);
    expect(book.bids[0].px).toBeLessThan(book.asks[0].px);
  });

  // ── 5. Status embeds maker key ─────────────────────────────

  test("/api/status embeds maker key", async ({ request }) => {
    const r = await request.get("/api/status");
    expect(r.ok()).toBe(true);
    const body = await r.json();
    expect(body).toHaveProperty("maker");
  });
});

// Five-minute soak: polls /api/processes every 5 s and FAILS
// if any RSX process restarts (pid changes) or uptime drops.
// Proves the F1 fix (CMP SO_REUSEPORT + graceful WAL drain on
// SIGTERM) keeps a warm cluster green with no operator action.
// Tagged @long so the fast lane can opt out via grep-invert.
test.describe("@long warm-cluster soak", () => {
  test.setTimeout(6 * 60 * 1000);

  test("system_stays_green_for_5m", async ({ request }) => {
    const DURATION_MS = 5 * 60 * 1000;
    const POLL_MS = 5_000;
    type Proc = { name: string; state: string; pid: number; uptime_s?: number };

    const firstResp = await request.get("/api/processes");
    expect(firstResp.ok()).toBe(true);
    const first: Proc[] = await firstResp.json();
    const baseline = new Map<string, Proc>();
    for (const p of first) {
      if (p.state === "running") baseline.set(p.name, p);
    }
    expect(baseline.size).toBeGreaterThanOrEqual(4);

    const deadline = Date.now() + DURATION_MS;
    let polls = 0;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, POLL_MS));
      const r = await request.get("/api/processes");
      expect(r.ok()).toBe(true);
      const cur: Proc[] = await r.json();
      for (const p of cur) {
        const base = baseline.get(p.name);
        if (!base) continue;
        expect(
          p.state,
          `${p.name} flipped to ${p.state} during soak`,
        ).toBe("running");
        expect(
          p.pid,
          `${p.name} pid changed ${base.pid} -> ${p.pid} (restart)`,
        ).toBe(base.pid);
        if (
          typeof p.uptime_s === "number" &&
          typeof base.uptime_s === "number"
        ) {
          expect(
            p.uptime_s,
            `${p.name} uptime decreased ${base.uptime_s} -> ${p.uptime_s}`,
          ).toBeGreaterThanOrEqual(base.uptime_s);
        }
      }
      polls += 1;
    }
    expect(polls).toBeGreaterThanOrEqual(50);
  });
});

// F20 regression: the gateway was reported to crash-loop under
// sustained WS connection churn. Investigation found no gw panic
// or leak -- the "restarts" were the process watcher respawning
// the estate during the F1 ME-restart cascade. With F1 fixed, gw-0
// is stable under churn. This guard drives WS churn (each order
// submit opens a fresh authed WS to the gateway) and asserts gw-0's
// pid never changes and its uptime climbs monotonically.
test.describe("@long gateway churn stability (F20)", () => {
  test.setTimeout(3 * 60 * 1000);

  test("gw-0 survives WS connection churn", async ({ request }) => {
    type Proc = { name: string; state: string; pid: number; uptime_s?: number };
    const gw = async (): Promise<Proc> => {
      const r = await request.get("/api/processes");
      expect(r.ok()).toBe(true);
      const procs: Proc[] = await r.json();
      const p = procs.find((x) => x.name === "gw-0" || x.name.startsWith("gw"));
      expect(p, "gw-0 not found").toBeTruthy();
      return p as Proc;
    };

    const base = await gw();
    expect(base.state).toBe("running");

    const CHURN_MS = 90_000;
    const deadline = Date.now() + CHURN_MS;
    let lastUptime = base.uptime_s ?? 0;
    let checks = 0;
    while (Date.now() < deadline) {
      // Each order submit goes through the gateway's authed WS path.
      await request
        .post("/api/orders?confirm=yes", {
          headers: { "x-confirm": "yes" },
          data: { symbol_id: SYMBOL_ID, side: 0, price: 50000, qty: 1000000 },
        })
        .catch(() => undefined);
      await new Promise((r) => setTimeout(r, 1500));
      const cur = await gw();
      expect(cur.state, "gw-0 not running during churn").toBe("running");
      expect(
        cur.pid,
        `gw-0 pid changed ${base.pid} -> ${cur.pid} (restart under churn)`,
      ).toBe(base.pid);
      if (typeof cur.uptime_s === "number") {
        expect(
          cur.uptime_s,
          `gw-0 uptime decreased ${lastUptime} -> ${cur.uptime_s}`,
        ).toBeGreaterThanOrEqual(lastUptime);
        lastUptime = cur.uptime_s;
      }
      checks += 1;
    }
    expect(checks).toBeGreaterThanOrEqual(30);
  });
});
