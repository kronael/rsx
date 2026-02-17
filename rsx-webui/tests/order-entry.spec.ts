/**
 * Visual regression + interaction tests for the OrderEntry component.
 *
 * Tests run against the built Vite app (vite preview). They verify:
 *   - Limit/Market tab switching
 *   - Buy/Sell side toggle
 *   - Price and Qty input fields
 *   - Leverage preset buttons
 *   - % qty buttons
 *   - Order cost preview row
 *   - TIF select (limit only)
 *   - Reduce-only / Post-only checkboxes
 *   - TP/SL inputs
 *   - Submit buttons
 *   - Visual snapshots at rest (limit/market)
 */
import { test, expect } from "@playwright/test";

test.describe("OrderEntry component", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await page.waitForSelector('[aria-label="Select trading pair"]', {
      state: "visible",
      timeout: 8000,
    });
    // Wait a tick for React state to settle
    await page.waitForTimeout(200);
  });

  // -------------------------------------------------------------------------
  // Tab switching
  // -------------------------------------------------------------------------

  test("Limit tab is active by default", async ({ page }) => {
    // 'Limit' tab should have tab-active class
    const limitBtn = page.getByRole("button", { name: /^Limit$/i }).first();
    await expect(limitBtn).toBeVisible();
    // Price input should be visible in limit mode
    await expect(
      page.getByPlaceholder("Price"),
    ).toBeVisible();
  });

  test("Market tab hides price input", async ({ page }) => {
    const marketBtn = page.getByRole("button", {
      name: /^Market$/i,
    }).first();
    await marketBtn.click();
    // Price input should disappear
    await expect(
      page.getByPlaceholder("Price"),
    ).not.toBeVisible();
  });

  test("switching back to Limit restores price input", async ({ page }) => {
    const marketBtn = page.getByRole("button", { name: /^Market$/i }).first();
    await marketBtn.click();
    const limitBtn = page.getByRole("button", { name: /^Limit$/i }).first();
    await limitBtn.click();
    await expect(page.getByPlaceholder("Price")).toBeVisible();
  });

  // -------------------------------------------------------------------------
  // Input fields
  // -------------------------------------------------------------------------

  test("price input accepts numeric text", async ({ page }) => {
    const priceInput = page.getByPlaceholder("Price");
    await priceInput.fill("45000.50");
    await expect(priceInput).toHaveValue("45000.50");
  });

  test("qty input accepts numeric text", async ({ page }) => {
    const qtyInput = page.getByPlaceholder("Qty");
    await qtyInput.fill("0.005");
    await expect(qtyInput).toHaveValue("0.005");
  });

  // -------------------------------------------------------------------------
  // Leverage presets
  // -------------------------------------------------------------------------

  test("leverage preset buttons are rendered", async ({ page }) => {
    for (const lv of ["1x", "10x", "100x"]) {
      await expect(
        page.getByRole("button", { name: lv }).first(),
      ).toBeVisible();
    }
  });

  test("clicking leverage button updates selection", async ({ page }) => {
    // Find 20x button and click
    const btn20 = page.getByRole("button", { name: "20x" }).first();
    await btn20.click();
    // After click the button should have the active accent style.
    // We check it has the expected class fragment (bg-accent).
    await expect(btn20).toHaveClass(/bg-accent/);
  });

  // -------------------------------------------------------------------------
  // % quantity buttons
  // -------------------------------------------------------------------------

  test("25% qty button is visible", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: "25%" }),
    ).toBeVisible();
  });

  test("100% qty button is visible", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: "100%" }),
    ).toBeVisible();
  });

  // -------------------------------------------------------------------------
  // Order cost preview
  // -------------------------------------------------------------------------

  test("Order Cost row is visible", async ({ page }) => {
    await expect(
      page.locator("text=Order Cost"),
    ).toBeVisible();
  });

  test("order cost updates when price and qty are filled", async ({
    page,
  }) => {
    await page.getByPlaceholder("Price").fill("50000");
    await page.getByPlaceholder("Qty").fill("0.01");
    // The cost cell should no longer show '--'
    const costVal = page.locator("text=Order Cost").locator("..").locator("span.font-mono");
    const text = await costVal.textContent();
    // Should now be a number (not '--')
    expect(text).not.toBe("--");
  });

  // -------------------------------------------------------------------------
  // Limit-only options
  // -------------------------------------------------------------------------

  test("TIF select is visible in limit mode", async ({ page }) => {
    await expect(page.locator("text=TIF")).toBeVisible();
    const sel = page.locator("select").first();
    await expect(sel).toBeVisible();
  });

  test("TIF select has GTC, IOC, FOK options", async ({ page }) => {
    const sel = page.locator("select").first();
    await expect(sel.locator("option[value='0']")).toBeAttached(); // GTC
    await expect(sel.locator("option[value='1']")).toBeAttached(); // IOC
    await expect(sel.locator("option[value='2']")).toBeAttached(); // FOK
  });

  test("reduce-only checkbox is present", async ({ page }) => {
    await expect(
      page.getByLabel("Reduce-only"),
    ).toBeAttached();
  });

  test("post-only checkbox is present in limit mode", async ({ page }) => {
    await expect(
      page.getByLabel("Post-only"),
    ).toBeAttached();
  });

  test("market mode hides TIF and post-only", async ({ page }) => {
    await page.getByRole("button", { name: /^Market$/i }).first().click();
    await expect(page.locator("text=TIF")).not.toBeVisible();
    await expect(page.getByLabel("Post-only")).not.toBeVisible();
  });

  // -------------------------------------------------------------------------
  // TP/SL inputs
  // -------------------------------------------------------------------------

  test("Take Profit input is visible", async ({ page }) => {
    await expect(
      page.getByLabel("Take profit price"),
    ).toBeVisible();
  });

  test("Stop Loss input is visible", async ({ page }) => {
    await expect(
      page.getByLabel("Stop loss price"),
    ).toBeVisible();
  });

  // -------------------------------------------------------------------------
  // Submit buttons
  // -------------------------------------------------------------------------

  test("Buy Limit and Sell Limit buttons are rendered", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /Buy Limit/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Sell Limit/i }),
    ).toBeVisible();
  });

  test("Buy Market and Sell Market buttons in market mode", async ({
    page,
  }) => {
    await page.getByRole("button", { name: /^Market$/i }).first().click();
    await expect(
      page.getByRole("button", { name: /Buy Market/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Sell Market/i }),
    ).toBeVisible();
  });

  test("submitting empty qty shows error", async ({ page }) => {
    const buyBtn = page.getByRole("button", {
      name: /Buy Limit/i,
    });
    await buyBtn.click();
    await expect(
      page.locator("text=Enter a valid quantity"),
    ).toBeVisible();
  });

  // -------------------------------------------------------------------------
  // Visual snapshots
  // -------------------------------------------------------------------------

  test("order entry panel snapshot — limit mode", async ({ page }) => {
    await page.waitForTimeout(200);
    // The order entry panel contains the Limit/Market tabs
    const panel = page
      .locator("div")
      .filter({ has: page.getByPlaceholder("Price") })
      .first();
    await expect(panel).toBeVisible();
    await expect(panel).toHaveScreenshot("order-entry-limit.png", {
      maxDiffPixelRatio: 0.02,
    });
  });

  test("order entry panel snapshot — market mode", async ({ page }) => {
    await page.getByRole("button", { name: /^Market$/i }).first().click();
    await page.waitForTimeout(150);
    const panel = page
      .locator("div")
      .filter({ has: page.getByPlaceholder("Qty") })
      .first();
    await expect(panel).toBeVisible();
    await expect(panel).toHaveScreenshot("order-entry-market.png", {
      maxDiffPixelRatio: 0.02,
    });
  });
});
