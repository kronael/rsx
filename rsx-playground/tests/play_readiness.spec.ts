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
