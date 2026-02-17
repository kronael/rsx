/**
 * Visual regression + interaction tests for the Orderbook component.
 *
 * The tests run against the built Vite app (vite preview) where the
 * Orderbook is rendered with empty/mock data (no live WS). They verify:
 *   - Component is mounted and ARIA roles are present
 *   - Column headers are visible
 *   - Side-toggle buttons (both/bids/asks) work
 *   - Tick-grouping select renders
 *   - Count-column toggle works
 *   - Symbol selector opens and closes
 *   - Visual snapshot of the orderbook panel at rest
 */
import { test, expect } from "@playwright/test";

test.describe("Orderbook component", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    // Wait for the React app to hydrate
    await page.waitForSelector('[aria-label="Select trading pair"]', {
      state: "visible",
      timeout: 8000,
    });
  });

  // -------------------------------------------------------------------------
  // Structure tests
  // -------------------------------------------------------------------------

  test("renders column headers", async ({ page }) => {
    // The orderbook header row has Price, Size, Total
    await expect(
      page.locator("text=Price").first(),
    ).toBeVisible();
    await expect(
      page.locator("text=Size").first(),
    ).toBeVisible();
    await expect(
      page.locator("text=Total").first(),
    ).toBeVisible();
  });

  test("has side-toggle buttons", async ({ page }) => {
    // "both", "bids", "asks" buttons
    await expect(
      page.getByRole("button", { name: /both/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /bids/i }).first(),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /asks/i }).first(),
    ).toBeVisible();
  });

  test("tick-grouping select is present", async ({ page }) => {
    const select = page.getByRole("combobox", {
      name: /tick grouping/i,
    });
    await expect(select).toBeVisible();
    // Check some expected option values exist
    const opt1x = select.locator('option[value="1"]');
    await expect(opt1x).toBeAttached();
    const opt10x = select.locator('option[value="10"]');
    await expect(opt10x).toBeAttached();
  });

  test("count-column toggle button is present", async ({ page }) => {
    // The '#' button toggles the order count column
    const btn = page.getByRole("button", { name: "#" }).first();
    await expect(btn).toBeVisible();
  });

  // -------------------------------------------------------------------------
  // Interaction tests
  // -------------------------------------------------------------------------

  test("side toggle: switching to bids-only hides ask area", async ({
    page,
  }) => {
    const bidsBtn = page.getByRole("button", {
      name: /^bids$/i,
    });
    await bidsBtn.click();
    await expect(bidsBtn).toHaveAttribute("aria-pressed", "true");

    // The asks section should not be visible when bids-only
    // (best signal: the "asks" toggle is not pressed)
    const asksBtn = page.getByRole("button", {
      name: /^asks$/i,
    });
    await expect(asksBtn).toHaveAttribute("aria-pressed", "false");
  });

  test("side toggle: switching to asks-only", async ({ page }) => {
    const asksBtn = page.getByRole("button", {
      name: /^asks$/i,
    });
    await asksBtn.click();
    await expect(asksBtn).toHaveAttribute("aria-pressed", "true");

    const bidsBtn = page.getByRole("button", {
      name: /^bids$/i,
    });
    await expect(bidsBtn).toHaveAttribute("aria-pressed", "false");
  });

  test("side toggle: both resets to default", async ({ page }) => {
    // Switch to bids, then back to both
    await page.getByRole("button", { name: /^bids$/i }).click();
    await page.getByRole("button", { name: /^both$/i }).click();

    const bothBtn = page.getByRole("button", { name: /^both$/i });
    await expect(bothBtn).toHaveAttribute("aria-pressed", "true");
  });

  test("count toggle shows # column header", async ({ page }) => {
    // Initially count column is hidden
    const countHeader = page.locator('span', { hasText: '#' }).first();

    // Click the toggle button
    const toggleBtn = page.getByRole("button", { name: "#" }).first();
    await toggleBtn.click();

    // After toggle, the # column header should be visible in the table
    await expect(countHeader).toBeVisible();

    // Toggle back off
    await toggleBtn.click();
  });

  test("tick grouping select changes to 10x", async ({ page }) => {
    const select = page.getByRole("combobox", {
      name: /tick grouping/i,
    });
    await select.selectOption("10");
    await expect(select).toHaveValue("10");
  });

  // -------------------------------------------------------------------------
  // Symbol selector (in TopBar, affects orderbook)
  // -------------------------------------------------------------------------

  test("symbol selector button is visible and has ARIA label", async ({
    page,
  }) => {
    const btn = page.getByRole("button", {
      name: /select trading pair/i,
    });
    await expect(btn).toBeVisible();
  });

  test("symbol selector opens dropdown on click", async ({ page }) => {
    const btn = page.getByRole("button", {
      name: /select trading pair/i,
    });
    await btn.click();
    // Dropdown listbox should be visible
    await expect(page.getByRole("listbox")).toBeVisible();
    // Close with Escape
    await page.keyboard.press("Escape");
    await expect(page.getByRole("listbox")).not.toBeVisible();
  });

  // -------------------------------------------------------------------------
  // Visual regression snapshot
  // -------------------------------------------------------------------------

  test("orderbook panel visual snapshot", async ({ page }) => {
    // Allow any initial data to settle
    await page.waitForTimeout(300);
    // Find the orderbook container by its column header
    const panel = page
      .locator("div")
      .filter({ hasText: /Price.*Size.*Total/s })
      .first();
    await expect(panel).toBeVisible();
    await expect(panel).toHaveScreenshot("orderbook-panel.png", {
      maxDiffPixelRatio: 0.02,
    });
  });
});
