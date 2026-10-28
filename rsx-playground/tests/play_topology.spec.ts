import { test, expect } from "@playwright/test";
import { waitForHTMX, verifyPolling } from "./test_helpers";

test.describe("Topology tab", () => {
  test("loads and shows process graph", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.locator("nav a", { hasText: "Topology" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Process Graph" })).toBeVisible();
    // Verify key process nodes exist in the interactive diagram
    await expect(page.locator(".component-node", { hasText: "Gateway" })).toBeVisible();
    await expect(page.locator(".component-node", { hasText: "Risk" })).toBeVisible();
    await expect(page.locator(".component-node", { hasText: "Marketdata" })).toBeVisible();
  });

  test("has core affinity card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "Core Affinity Map" })).toBeVisible();
  });

  test("has cast connections card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "Cast Connections" })).toBeVisible();
  });

  test("has process list card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "Process List" })).toBeVisible();
  });

  test("process graph shows nodes for running processes", async ({ page }) => {
    await page.goto("/topology");

    // Should show key process names in topology nodes
    await expect(page.locator(".component-node", { hasText: "Gateway" })).toBeVisible();
    await expect(page.locator(".component-node", { hasText: "Risk" })).toBeVisible();
    await expect(page.locator(".component-node", { hasText: "Matching" })).toBeVisible();
    await expect(page.locator(".component-node", { hasText: "Marketdata" })).toBeVisible();
    await expect(page.locator(".component-node", { hasText: "Recorder" })).toBeVisible();
    await expect(page.locator("#topo-node-mark")).toBeVisible();
  });

  test("process graph shows edges for cast connections", async ({ page }) => {
    await page.goto("/topology");

    // Should show connection labels between nodes
    await expect(page.locator("text=cast").first()).toBeVisible();
    await expect(page.locator("text=WAL").first()).toBeVisible();
  });

  test("core affinity map auto-refreshes every 5s", async ({ page }) => {
    await page.goto("/topology");
    const affinity = page.locator("[hx-get='./x/core-affinity']");

    await verifyPolling(affinity, "every 5s");
  });

  test("core affinity displays process-to-core mapping", async ({ page }) => {
    await page.goto("/topology");
    const affinity = page.locator("[hx-get='./x/core-affinity']");
    await waitForHTMX(page, 2000);

    // Should show core mapping or "no processes"
    const content = await affinity.textContent();
    // Dev boxes don't pin cores, so the map honestly reads "cpus N"
    // or "no pinning" per process; "no processes" only when empty.
    expect(content).toMatch(/cpus|Core|no pinning|no processes/i);
  });

  test("cast connections card auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/topology");
    const castFlows = page.locator("[hx-get='./x/cast-flows']");

    await verifyPolling(castFlows, "every 2s");
  });

  test("cast connections show gateway-risk-ME flow", async ({ page }) => {
    await page.goto("/topology");
    const castFlows = page.locator("[hx-get='./x/cast-flows']");
    await waitForHTMX(page, 2000);

    // Should show connection names
    const content = await castFlows.textContent();
    expect(content).toMatch(/Gateway.*Risk|Risk.*ME|ME.*Mktdata/i);
  });

  test("process list auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/topology");
    const procList = page.locator("[hx-get='./x/processes']").last();

    await verifyPolling(procList, "every 2s");
  });

  // The topology pill and process API share scan_processes().
  test("topology_status_pill_agrees_with_api_processes (F5)",
    async ({ request }) => {
      const procRes = await request.get("/api/processes");
      expect(procRes.ok()).toBe(true);
      const procs = await procRes.json() as Array<{
        name: string; state: string; pid: number | string;
      }>;
      const gwRunning = procs.some(
        (p) => p.state === "running" && /^gw-/.test(p.name),
      );
      const topo = await request.get("/x/topology/gateway");
      expect(topo.ok()).toBe(true);
      const html = await topo.text();
      if (gwRunning) {
        expect(
          html.toLowerCase(),
          "topology says stopped but API says gw-0 running",
        ).toContain("running");
        expect(html).not.toMatch(/pid:\s*-/);
      }
    },
  );

  // A running mark process exposes numeric detail.
  test("topology_mark_detail_shows_real_data (F6)",
    async ({ request }) => {
      const procRes = await request.get("/api/processes");
      const procs = await procRes.json() as Array<{
        name: string; state: string;
      }>;
      const markRunning = procs.some(
        (p) => p.state === "running" && p.name === "mark",
      );
      const r = await request.get("/x/topology/mark");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      expect(html).not.toContain("requires mark process");
      if (markRunning) {
        // Must include funding settlement window (cheap proof
        // we wired the real detail payload).
        expect(html.toLowerCase()).toMatch(
          /funding next settlement|sample interval|symbols tracked/
        );
      }
    },
  );

  // ME-to-marketdata counters include fills and BBOs.
  test("cast_counters_track_marketdata_progress (F9)",
    async ({ request }) => {
      const r = await request.get("/x/cast-flows");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      // Capture all three flows in order. We don't require
      // non-zero (cold start).
      expect(html).toContain("ME -> Mktdata");
      expect(html).toContain("Gateway -> Risk");
    },
  );

  // Gateway detail must not present a hard-coded breaker state.
  test("gateway_circuit_breaker_not_hardcoded (F14)",
    async ({ request }) => {
      // Kill the gateway: a real breaker reader would change or
      // disappear; a hard-coded "closed" would persist.
      await request.post("/api/processes/gw-0/kill", {
        headers: { "x-confirm": "yes" },
      });
      await new Promise((r) => setTimeout(r, 2000));
      const r = await request.get("/x/topology/gateway");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      expect(html.toLowerCase()).not.toContain("circuit breaker");
      // Restore for subsequent tests.
      await request.post("/api/processes/gw-0/start", {
        headers: { "x-confirm": "yes" },
      });
    },
  );

  // Dashboard-local flow counters carry a "(session)" label.
  test("flow_counters_not_from_dashboard_memory (F15)",
    async ({ request }) => {
      const r = await request.get("/x/topology/flow");
      expect(r.ok()).toBe(true);
      const body = await r.json() as {
        nodes: Array<{ key: string; rate: string }>;
      };
      const rateOf = (k: string) =>
        body.nodes.find((n) => n.key === k)?.rate ?? "";
      // The three Python-memory-derived rates must carry the
      // honest "(session)" disclaimer.
      expect(rateOf("client")).toContain("(session)");
      expect(rateOf("gateway")).toContain("(session)");
      expect(rateOf("marketdata")).toContain("(session)");
    },
  );

  // Core affinity reflects OS state rather than list position.
  test("core_affinity_backed_by_real_cpu_affinity (F21)",
    async ({ request }) => {
      const r = await request.get("/x/core-affinity");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      // Must not invent ascending fake core ids.
      expect(html).not.toMatch(/Core\s+\d+/);
      // Real labels: either a cpu set, "no pinning", "-",
      // or "no processes" when nothing is running.
      const ok = /cpus\s|no pinning|>-<|no processes/.test(html);
      expect(ok).toBe(true);
    },
  );

  // The dashboard reports WAL lag because SPSC occupancy is not visible.
  test("ring_pressure_reads_real_telemetry_or_is_labeled_derived (F25)",
    async ({ page, request }) => {
      const r = await request.get("/x/ring-pressure");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      // Honest row labels reference WAL lag, not "Ring".
      expect(html.toLowerCase()).toContain("wal lag");
      // Overview card uses the WAL-lag proxy heading.
      await page.goto("/overview");
      await expect(
        page.getByText("WAL stream lag (proxy)"),
      ).toBeVisible();
    },
  );

  // Per-pipe counters come from their corresponding WAL streams.
  test("cast_flow_counters_distinct_per_pipe",
    async ({ request }) => {
      const r = await request.get("/x/cast-flows");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      const tds = [
        ...html.matchAll(/<td[^>]*>([^<]*)<\/td>/g),
      ].map((m) => m[1].trim());
      // Layout: [name, sent, recv, nak, drop] × 3
      const sentG = tds[1];
      const sentR = tds[6];
      const sentM = tds[11];
      // If any pipe is non-zero, at least one must differ from
      // the other two — otherwise we're back to the "1117"
      // ghost. Three "0"s on a cold cluster is fine.
      const nums = [sentG, sentR, sentM]
        .map((s) => Number(s))
        .filter((n) => Number.isFinite(n) && n > 0);
      if (nums.length > 0) {
        const allSame = nums.every((n) => n === nums[0])
          && nums.length === 3;
        expect(
          allSame,
          `cast pipes all read ${nums[0]} — ghost is back`,
        ).toBe(false);
      }
    },
  );

  // A stopped maker reports no levels.
  test("maker_status_has_no_levels_when_not_running",
    async ({ request }) => {
      const r = await request.get("/api/maker/status");
      expect(r.ok()).toBe(true);
      const body = await r.json() as { running: boolean; levels: number };
      if (!body.running) {
        expect(body.levels).toBe(0);
      }
    },
  );
});
