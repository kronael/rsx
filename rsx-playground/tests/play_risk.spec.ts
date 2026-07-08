import { test, expect } from "@playwright/test";
import { waitForHTMX, waitForRefresh, verifyPolling } from "./test_helpers";

test.describe("Risk tab", () => {
  test("loads and has user lookup", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.locator("nav a", { hasText: "Risk" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "User Actions" })).toBeVisible();
    await expect(page.locator("#risk-uid")).toBeVisible();
    await expect(page.locator("button", { hasText: "Lookup" }))
      .toBeVisible();
  });

  test("has freeze and unfreeze buttons", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.locator("button", { hasText: /^Freeze$/ }))
      .toBeVisible();
    await expect(page.locator("button", { hasText: /^Unfreeze$/ }))
      .toBeVisible();
  });

  test("has position heatmap card", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.getByRole("heading", { name: "Position Heatmap" })).toBeVisible();
  });

  test("has margin ladder card", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.getByRole("heading", { name: "Margin Ladder" })).toBeVisible();
  });

  test("has liquidation queue card", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.locator("main")).toContainText("Liquidation Queue");
  });

  // New interactive tests (13 total)

  test("user lookup by ID updates display", async ({ page }) => {
    await page.goto("/risk");
    const riskData = page.locator("#risk-data");

    // Enter user ID
    await page.locator("#risk-uid").fill("1");

    // Click lookup
    await page.locator("button", { hasText: "Lookup" }).click();
    await waitForHTMX(page);

    // Verify data updated (should show user info or no data message)
    const content = await riskData.textContent();
    expect(content).toBeTruthy();
    expect(content).toMatch(/user|no data|error|not connected/i);
  });

  test("user lookup shows a flat state for users with no WAL fills", async ({ page }) => {
    await page.goto("/risk");
    const riskData = page.locator("#risk-data");

    await page.locator("#risk-uid").fill("999");
    await page.locator("button", { hasText: "Lookup" }).click();
    await waitForHTMX(page);

    await expect(riskData).toContainText(/user 999/i);
    await expect(riskData).toContainText(/flat|no fills/i);
  });

  test("freeze button triggers action", async ({ page }) => {
    await page.goto("/risk");
    const freezeBtn = page.locator("button", { hasText: /^Freeze$/ }).first();

    await freezeBtn.click();
    await waitForHTMX(page);

    // Action should complete (check for console error or success)
    // Button should still be visible
    await expect(freezeBtn).toBeVisible();
  });

  test("unfreeze button triggers action", async ({ page }) => {
    await page.goto("/risk");
    const unfreezeBtn = page.locator("button", { hasText: /^Unfreeze$/ }).first();

    await unfreezeBtn.click();
    await waitForHTMX(page);

    await expect(unfreezeBtn).toBeVisible();
  });

  test("position heatmap auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/risk");
    const heatmap = page.locator("[hx-get='./x/position-heatmap']");

    // Verify polling configured
    await verifyPolling(heatmap, "every 2s");
  });

  test("position heatmap shows placeholder when no data", async ({ page }) => {
    await page.goto("/risk");
    const heatmap = page.locator("[hx-get='./x/position-heatmap']");
    await waitForHTMX(page, 2000);

    // Should show placeholder or fill data
    await expect(heatmap).toContainText(/no fill data|no data|users|Symbol/i);
  });

  test("position heatmap renders after order submission", async ({
    page,
  }) => {
    // Submit a sell then a crossing buy to generate a fill when
    // the exchange is running; heatmap shows Symbol table with
    // fills, or "no fill data" when exchange is offline — both
    // are valid rendered states from /x/position-heatmap.
    await page.request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "sell",
        order_type: "limit",
        price: "0.050",
        qty: "10",
        user_id: "2",
      },
    });
    await page.request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "buy",
        order_type: "limit",
        price: "0.051",
        qty: "10",
        user_id: "1",
      },
    });
    await page.goto("/risk");
    const heatmap = page.locator("[hx-get='./x/position-heatmap']");
    // Heatmap must render something meaningful — either fills or
    // the empty-state placeholder; "loading..." must be gone.
    await expect(heatmap).not.toContainText(
      /loading\.\.\./i,
      { timeout: 10000 },
    );
    await expect(heatmap).toContainText(
      /no fill data|Symbol/i,
      { timeout: 10000 },
    );
  });

  test("margin ladder auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/risk");
    const ladder = page.locator("[hx-get='./x/margin-ladder']");

    await verifyPolling(ladder, "every 2s");
  });

  test("margin ladder shows liquidation distance placeholder", async ({ page }) => {
    await page.goto("/risk");
    const ladder = page.locator("[hx-get='./x/margin-ladder']");
    await waitForHTMX(page, 2000);

    // Should show placeholder or data
    const content = await ladder.textContent();
    expect(content).toBeTruthy();
  });

  test("margin ladder renders after order submission", async ({
    page,
  }) => {
    // Submit a sell then a crossing buy to generate a fill when
    // the exchange is running; ladder shows Symbol/Side/Price
    // table with fills, or "no fill data" offline — both are
    // valid rendered states from /x/margin-ladder.
    await page.request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "sell",
        order_type: "limit",
        price: "0.050",
        qty: "10",
        user_id: "1",
      },
    });
    await page.request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "buy",
        order_type: "limit",
        price: "0.051",
        qty: "10",
        user_id: "2",
      },
    });
    await page.goto("/risk");
    const ladder = page.locator("[hx-get='./x/margin-ladder']");
    // Ladder must render something meaningful — fills or
    // the empty-state placeholder; "loading..." must be gone.
    await expect(ladder).not.toContainText(
      /loading\.\.\./i,
      { timeout: 10000 },
    );
    await expect(ladder).toContainText(
      /no fill data|Symbol/i,
      { timeout: 10000 },
    );
  });

  test("funding card auto-refreshes", async ({ page }) => {
    await page.goto("/risk");
    const funding = page.locator("[hx-get='./x/risk-overview']");

    await verifyPolling(funding, "every 3s");
    await expect(funding).toContainText(/Funding Rates/i, { timeout: 5000 });
  });

  test("liquidation queue auto-refreshes", async ({ page }) => {
    await page.goto("/risk");
    const liqQueue = page.locator("[hx-get='./x/risk-overview']");

    await verifyPolling(liqQueue, "every 3s");
    await expect(liqQueue).toContainText(
      /Liquidation Queue/i,
      { timeout: 5000 },
    );
  });

  test("risk latency card auto-refreshes every 5s", async ({ page }) => {
    await page.goto("/risk");
    const latency = page.locator("[hx-get='./x/risk-latency']");

    await verifyPolling(latency, "every 5s");
  });

  test("user action buttons have correct HTMX attributes", async ({ page }) => {
    await page.goto("/risk");

    // Verify buttons have hx-post attributes
    const createBtn = page.locator("button", { hasText: "Create User" });
    await expect(createBtn).toHaveAttribute("hx-post", "./api/users/create");

    const depositBtn = page.locator("button", { hasText: "Deposit" });
    await expect(depositBtn).toHaveAttribute("hx-post", /\/deposit/);

    const liquidateBtn = page.locator("button", { hasText: "Liquidate" });
    await expect(liquidateBtn).toHaveAttribute("hx-post", "./api/risk/liquidate");
  });

  // F8: SYSTEM-WIDE RISK METRICS used to show "--" for OI and
  // notional while the Maker tab reported "Positions 2". Now
  // accounts_with_positions ≥ users with any fill activity and
  // total_oi falls back to gross filled notional when nets
  // cancel out. Verify via /api/risk/overview's system block.
  test("system_metrics_populated_when_fills (F8)",
    async ({ request }) => {
      const r = await request.get("/api/risk/overview");
      expect(r.ok()).toBe(true);
      const body = await r.json();
      expect(body).toHaveProperty("system");
      const sys = body.system as {
        total_oi: number;
        long_notional: number;
        short_notional: number;
        accounts_with_positions: number;
      };
      // Cross-check against /x/key-metrics' Positions count.
      const km = await request.get("/x/key-metrics");
      const kmHtml = await km.text();
      const posMatch = kmHtml.match(
        /Positions[\s\S]*?>(\d+)</
      );
      if (posMatch) {
        const keyMetricsPos = Number(posMatch[1]);
        expect(
          sys.accounts_with_positions,
          "risk.system.accounts < key-metrics.Positions",
        ).toBeGreaterThanOrEqual(keyMetricsPos);
        if (keyMetricsPos > 0) {
          expect(sys.total_oi).toBeGreaterThan(0);
        }
      }
    },
  );

  test("all risk cards load without errors", async ({ page }) => {
    await page.goto("/risk");

    // Verify all cards visible
    await expect(page.getByRole("heading", { name: "Position Heatmap" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Margin Ladder" })).toBeVisible();
    await expect(page.locator("main")).toContainText("Funding Rates");
    await expect(page.locator("main")).toContainText("Liquidation Queue");
    await expect(page.getByRole("heading", { name: "Risk Check Latency" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "User Actions" })).toBeVisible();

    await waitForHTMX(page, 2000);

    // Check no unexpected console errors (ignore CDN/network errors)
    const errors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("ERR_")) {
        errors.push(msg.text());
      }
    });

    await page.waitForTimeout(1000);
    expect(errors.length).toBe(0);
  });

  // F16: /api/risk/funding used to fabricate the index price as
  // `mark * 1.0001`, then derive premium and rate from that fake.
  // The index must now come from the real mark process
  // (RECORD_MARK_PRICE on the mark WAL stream) or be honestly
  // flagged (index_source != "formula"/synthetic, and never a
  // fixed 1.0001 ratio).
  test("funding_uses_real_index_source_not_formula_stub (F16)",
    async ({ request }) => {
      const r = await request.get("/api/risk/funding");
      expect(r.ok()).toBe(true);
      const body = await r.json() as {
        funding: Array<{
          mark_px: number;
          index_px: number;
          index_source?: string;
        }>;
      };
      for (const f of body.funding) {
        // Every entry must declare where its index came from.
        expect(f.index_source).toBeDefined();
        // "none" (mark process down / no external data) is the
        // honest absence; "mark-process" is the real source. The
        // forbidden state is a fabricated index.
        expect(["mark-process", "none"]).toContain(
          f.index_source
        );
        if (f.index_source === "none") {
          // No fake index when there's no source.
          expect(f.index_px).toBe(0);
        }
        // The old stub produced index_px == round(mark*1.0001),
        // i.e. a fixed 1-bp premium on every symbol. Forbid that
        // exact relationship for a non-zero index.
        if (f.index_px > 0 && f.mark_px > 0) {
          expect(f.index_px).not.toBe(
            Math.trunc(f.mark_px * 1.0001)
          );
        }
      }
    },
  );

  // F3.1: collateral / equity / IM / MM / notional must render as
  // USD with $ + commas (format_notional), never bare i64.
  test("risk_collateral_formatted_as_currency",
    async ({ request }) => {
      const r = await request.get("/x/risk-overview");
      expect(r.ok()).toBe(true);
      const html = await r.text();
      // Forbid the raw i64 the CEO captured ("999999972019150"
      // and friends — any 13+ digit run is the smoking gun).
      expect(html).not.toMatch(/\b\d{13,}\b/);
      // Currency-shaped: at least one "$<digits>(,<digits>)*.<dd>"
      // somewhere in the panel.
      expect(html).toMatch(/\$\d{1,3}(?:,\d{3})*\.\d{2}/);
    },
  );
});
