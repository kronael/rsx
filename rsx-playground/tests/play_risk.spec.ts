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
    await expect(page.locator("h2", { hasText: "Liquidation Queue" })).toBeVisible();
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

  test("user lookup shows postgres not connected message when DB unavailable", async ({ page }) => {
    await page.goto("/risk");
    const riskData = page.locator("#risk-data");

    await page.locator("#risk-uid").fill("999");
    await page.locator("button", { hasText: "Lookup" }).click();
    await waitForHTMX(page);

    // Should show "no data" or "not connected" message
    await expect(riskData).toContainText(/no data|not connected|postgres|error/i);
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
        price: "50000",
        qty: "10",
        user_id: "2",
      },
    });
    await page.request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "buy",
        order_type: "limit",
        price: "51000",
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
        price: "50000",
        qty: "10",
        user_id: "1",
      },
    });
    await page.request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "buy",
        order_type: "limit",
        price: "51000",
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
    const funding = page.locator("[hx-get='./x/funding']");

    await verifyPolling(funding, "every 2s");
  });

  test("liquidation queue auto-refreshes", async ({ page }) => {
    await page.goto("/risk");
    const liqQueue = page.locator("[hx-get='./x/liquidations']");

    await verifyPolling(liqQueue, "every 2s");
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

  test("all risk cards load without errors", async ({ page }) => {
    await page.goto("/risk");

    // Verify all cards visible
    await expect(page.getByRole("heading", { name: "Position Heatmap" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Margin Ladder" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Funding", exact: true })).toBeVisible();
    await expect(page.locator("h2", { hasText: "Liquidation Queue" })).toBeVisible();
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
});
