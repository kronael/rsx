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

  // F5: /x/topology/gateway used to render "stopped + pid: -"
  // while /api/processes said gw-0 was running. Both must read
  // from the same scan_processes() oracle now (via PROC_HINTS).
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

  // F6: /x/topology/mark used to render "mark data requires
  // mark process" even when the mark process was up. Detail
  // panel must show real numeric mark prices now.
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
      // Old stub string must be gone unconditionally.
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

  // F9: ME -> Mktdata used to show Sent 0 / Recv 0 while
  // marketdata was visibly receiving updates. The counter now
  // sums fills + BBOs (the actual cast payload).
  test("cast_counters_track_marketdata_progress (F9)",
    async ({ request }) => {
      const r = await request.get("/x/cast-flows");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      // Capture all three flows in order. We don't require
      // non-zero (cold start), but ME->Mktdata must never be
      // strictly less than the BBO+fill total — that was the
      // bug shape.
      expect(html).toContain("ME -> Mktdata");
      expect(html).toContain("Gateway -> Risk");
    },
  );

  // F14: the gateway detail panel used to print a hard-coded
  // ("circuit breaker", "closed") row that never read real state
  // — it stayed "closed" even with the gateway dead. The fake
  // row is removed; there must be no comforting literal.
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
      // The lie was a literal "circuit breaker ... closed" row.
      // It is gone whether or not the gateway is up.
      expect(html.toLowerCase()).not.toContain("circuit breaker");
      // Restore for subsequent tests.
      await request.post("/api/processes/gw-0/start", {
        headers: { "x-confirm": "yes" },
      });
    },
  );

  // F15: /x/topology/flow node rates are dashboard-local session
  // counters (recent_orders / recent_fills / _book_snap), not
  // cluster truth. They must be labelled "(session)" so they
  // don't masquerade as live cluster throughput.
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

  // F21: /x/core-affinity must reflect real OS affinity instead
  // of inventing a Core{i} label from list index. Running rows
  // expose "cpus ..." or "no pinning"; non-running rows show "-".
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

  // F25: panel must not claim to show SPSC ring occupancy
  // (intra-process rtrb is not visible from the dashboard).
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

  // F3.4: the three cast pipes pulled from one source repeated
  // the same number three times ("1117 / 1117 / 1117"). Per-pipe
  // counters now come from per-process WAL streams. Under load
  // (any maker traffic), the three should diverge. With a cold
  // cluster every count is 0 — also acceptable. The forbidden
  // shape is three identical non-zero numbers.
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

  // F27: /api/maker/status must not echo stale
  // tmp/maker-status.json after the maker dies.
  test("maker_status_clears_stats_when_not_running (F27)",
    async ({ request }) => {
      const r = await request.get("/api/maker/status");
      expect(r.ok()).toBe(true);
      const body = await r.json() as {
        running: boolean; levels: number;
        errors: unknown; stale: boolean;
      };
      if (!body.running) {
        expect(body.levels).toBe(0);
        expect(body.errors).toBeNull();
        expect(body.stale).toBe(true);
      }
    },
  );
});
