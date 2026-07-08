import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { waitForHTMX } from "./test_helpers";

// ── Layer 3: INTERACTION ────────────────────────────────────
// The old suite clicked a button and asserted the button was still
// visible. It never asserted the EFFECT of the click. So WAL Verify
// (hx-swap=none, no output), the dump OOM, the risk-lookup-ignores-
// user_id bug, and the server-log-noise default all passed green.
// This layer clicks and asserts the resulting content.
//
// Live-cluster-dependent: skips if /healthz is unreachable (mirrors
// the existing e2e gate).

async function clusterUp(req: APIRequestContext): Promise<number> {
  // Retry a couple of times: a single transient /healthz blip (the
  // server is busy right after a heavy WAL dump) must not false-skip.
  for (let i = 0; i < 3; i++) {
    try {
      const r = await req.get("/healthz", { timeout: 4000 });
      if (r.ok()) return (await r.json()).processes_running ?? 0;
    } catch { /* retry */ }
    await new Promise((res) => setTimeout(res, 500));
  }
  return 0;
}

// Extract the running-process numerator from a surface's text. Strips
// tags first — the count is often split across <span>s (proc</span>
// <span>7/7</span>), so a regex on raw HTML would miss it.
function runningCount(html: string, anchor: RegExp): number | null {
  const text = html.replace(/<[^>]+>/g, " ");
  const m = text.match(anchor);
  return m ? parseInt(m[1], 10) : null;
}

test.describe("Interaction effects", () => {
  test.beforeEach(async ({ request }) => {
    test.skip((await clusterUp(request)) < 6, "cluster not up (need 6+)");
  });

  // ── FINDING #8: WAL Verify had hx-swap=none → zero feedback.
  test("WAL Verify renders a 'verified' result", async ({ page }) => {
    await page.goto("/wal");
    const result = page.locator("#wal-tools-result");
    await page.locator("button", { hasText: "Verify" }).click();
    await expect(result).toContainText(/verified/i, { timeout: 8000 });
  });

  // ── FINDING #7: WAL Dump OOM'd on the whole archive + rendered
  // nothing. Now it must render actual records, never an OOM.
  test("WAL Dump renders records, not an OOM/empty", async ({ page }) => {
    await page.goto("/wal");
    const result = page.locator("#wal-tools-result");
    await page.locator("button", { hasText: /Dump/ }).click();
    await expect(result).toContainText(
      /seq|ORDER_ACCEPTED|BBO|FILL|\.wal|record/i, { timeout: 8000 });
    const txt = (await result.innerText()).toLowerCase();
    expect(txt).not.toContain("out of memory");
    expect(txt).not.toContain("memoryerror");
    expect(txt).not.toContain("traceback");
  });

  // ── FINDING #2: Risk Lookup ignored user_id (every id → same row).
  test("Risk Lookup differs by user_id", async ({ page }) => {
    await page.goto("/risk");
    const data = page.locator("#risk-data");
    const uid = page.locator("#risk-uid");
    const lookup = page.locator("button", { hasText: "Lookup" });

    // Wait for the SPECIFIC user labels so a stale "flat/no fills"
    // response from the previous lookup cannot satisfy the next wait.
    await uid.fill("1");
    await lookup.click();
    await expect(data).toContainText(/user 1/i, { timeout: 4000 });
    const one = (await data.innerText()).trim();

    await uid.fill("4242");
    await lookup.click();
    await expect(data).toContainText(/user 4242/i, { timeout: 4000 });
    await expect(data).toContainText(/flat|no fills/i, { timeout: 4000 });
    const other = (await data.innerText()).trim();

    expect(other).not.toBe(one);
  });

  // ── FINDING #25: Logs default = self-noise ([server] access logs);
  // per-source pill must isolate exactly that source.
  test("Log source pills isolate their source; default hides [server]",
    async ({ page }) => {
      // Seed traffic so gateway/risk/me/marketdata all emit fresh log
      // lines (their tails can be sparse right after a restart) — makes
      // the isolation check below non-vacuous and deterministic.
      for (let i = 0; i < 4; i++) {
        await page.request.post("/api/orders/test", {
          form: {
            symbol_id: "10", side: "buy", price: "0", qty: "10",
            tif: "IOC", user_id: "1",
          },
        });
      }
      await page.goto("/logs");
      await waitForHTMX(page, 2000);

      // Disable the live tail: it prepends UNFILTERED rows, and an
      // in-flight tail fetch can land after a pill swap and contaminate
      // the filtered view (a benign UI race). Off = deterministic filter.
      await page.locator("#tail-toggle").click();
      await expect(page.locator("#tail-toggle")).toContainText("tail: off");
      await page.waitForTimeout(2500); // drain any in-flight tail fetch

      // Distinct source-column values of the REAL log rows. A real row
      // has 4 <td>s (source/time/level/msg); the empty-view placeholder
      // is a single colspan cell ("no log lines") — skip it so an empty
      // filtered view reads as [] (not a bogus "no log lines" source).
      const sources = async (): Promise<string[]> => {
        const rows = page.locator("#log-table tr");
        const n = await rows.count();
        const out: string[] = [];
        for (let i = 0; i < n; i++) {
          const tds = rows.nth(i).locator("td");
          if ((await tds.count()) < 4) continue;
          const t = (await tds.first().innerText()).trim();
          if (t && t !== "—") out.push(t);
        }
        return Array.from(new Set(out));
      };

      // Click a pill and wait for the table to actually REFLECT that
      // source (the HTMX swap is async — reading too early sees the
      // previous pill's rows). `ok` decides when the settled view is
      // acceptable. A genuinely broken filter (shows every source) never
      // settles to all-match → poll times out → the test fails.
      const clickPillUntil = async (
        id: string, ok: (srcs: string[]) => boolean,
      ) => {
        await page.locator(`#${id}`).click();
        await expect.poll(async () => ok(await sources()),
          { timeout: 5000 }).toBe(true);
      };

      // Positive isolation: each pill shows only its own source.
      const pills: [string, RegExp][] = [
        ["fp-gateway", /^gw/],
        ["fp-risk", /^risk/],
        ["fp-matching", /^me/],
        ["fp-marketdata", /^(marketdata|mktdata)/],
        ["fp-mark", /^mark(-|$)/],
        ["fp-recorder", /^recorder/],
        ["fp-maker", /^maker/],
      ];
      let sawRows = 0;
      for (const [id, re] of pills) {
        await clickPillUntil(id, (s) => s.every((x) => re.test(x)));
        const srcs = await sources();
        for (const s of srcs) {
          expect(s, `pill ${id} leaked source "${s}"`).toMatch(re);
        }
        if (srcs.length) sawRows++;
      }
      // The filter is only meaningful if some sources actually produced
      // rows (a live cluster must log to at least a couple of these).
      expect(sawRows).toBeGreaterThanOrEqual(2);

      // FINDING #25: default "all" view must NOT include the dashboard's
      // own [server] access-log noise...
      await clickPillUntil("fp-all", (s) => s.length > 0);
      const allSrcs = await sources();
      expect(allSrcs).not.toContain("server");

      // The server pill must not leak other sources. Some runs have no
      // server log rows in the bounded tail, so an empty settled view is OK.
      await clickPillUntil("fp-server",
        (s) => s.every((x) => x.startsWith("server")));
      const serverSrcs = await sources();
      expect(serverSrcs.every((s) => s.startsWith("server"))).toBe(true);
    });

  // ── FINDING #11: process count reported 4 ways (7/7, 6/6, 7/6...).
  // nav chip, pulse bar, key-metrics, and Verify must AGREE.
  test("process-count agrees across chip/pulse/key-metrics/verify",
    async ({ request }) => {
      const chip = await (await request.get("/x/proc-chip")).text();
      const pulse = await (await request.get("/x/pulse")).text();
      const km = await (await request.get("/x/key-metrics")).text();
      const verify = await (await request.get("/x/verify")).text();

      const c = runningCount(chip, /procs\s*(\d+)\/(\d+)/i);
      const p = runningCount(pulse, /proc\s+(\d+)\/(\d+)/i);
      const k = runningCount(km, /Processes\s*(\d+)\/(\d+)/i);
      const v = runningCount(verify, /processes running\D*(\d+)\/(\d+)/i);

      expect(c, "proc-chip count").not.toBeNull();
      expect(p, "pulse count").not.toBeNull();
      expect(k, "key-metrics count").not.toBeNull();
      expect(v, "verify count").not.toBeNull();
      expect(
        new Set([c, p, k, v]).size,
        `counts disagree: chip=${c} pulse=${p} key-metrics=${k} verify=${v}`,
      ).toBe(1);
    });
});

// ── Start/Stop All full cycle (destructive) ─────────────────
// Stops the whole cluster and brings it back with a chosen scenario.
// Tagged @long so it is excluded from the quick verification run
// (--grep-invert @long) and only runs in a dedicated lane. Restores
// the baseline (minimal + maker → 7/7) in afterAll no matter what.
test.describe("Start/Stop All @long", () => {
  test.skip(process.env.PW_LONG !== "1", "set PW_LONG=1 to run long destructive flows");

  async function healthz(req: APIRequestContext): Promise<number> {
    return clusterUp(req);
  }
  async function waitCount(
    req: APIRequestContext, pred: (n: number) => boolean, ms: number,
  ): Promise<number> {
    const end = Date.now() + ms;
    let last = -1;
    while (Date.now() < end) {
      last = await healthz(req);
      if (pred(last)) return last;
      await new Promise((r) => setTimeout(r, 2000));
    }
    return last;
  }

  test.afterAll(async ({ request }) => {
    // Always restore the baseline cluster (minimal scenario → 7/7).
    await request.post("/api/processes/all/start", {
      form: { "scenario-ov": "minimal" }, timeout: 180_000,
    });
    await waitCount(request, (n) => n >= 6, 180_000);
  });

  test("@long stop-all → 0, start non-minimal → up, restore",
    async ({ page, request }) => {
      test.setTimeout(300_000);
      test.skip((await clusterUp(request)) < 1, "cluster unreachable");

      // Self-establish a clean baseline first so we can't cascade-fail.
      await page.goto("/overview");
      await page.locator("button", { hasText: "Stop All" }).click();
      expect(await waitCount(request, (n) => n === 0, 60_000)).toBe(0);

      // Start a NON-minimal scenario and assert it actually comes up.
      await page.goto("/overview");
      await page.locator("input[name='scenario-ov'][value='full']")
        .check({ force: true });
      await page.locator("button", { hasText: "Build & Start All" })
        .click();
      const up = await waitCount(request, (n) => n >= 6, 180_000);
      expect(up, "cluster did not reach 6 running").toBeGreaterThanOrEqual(6);

      // The active scenario must reflect the chosen (non-minimal) one.
      const scen = await (await request.get("/x/current-scenario")).text();
      expect(scen).toContain("full");
    });
});
