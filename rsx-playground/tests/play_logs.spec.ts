import { test, expect } from "@playwright/test";
import { waitForHTMX, waitForRefresh, pressSlashForSearch, pressClearShortcut } from "./test_helpers";

test.describe("Logs tab", () => {
  test("loads and has filters", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.locator("nav a", { hasText: "Logs" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Unified Log" })).toBeVisible();
    await expect(page.locator("#log-process")).toBeVisible();
    await expect(page.locator("#log-level")).toBeVisible();
    await expect(page.locator("#log-search")).toBeVisible();
  });

  test("process filter has expected options", async ({ page }) => {
    await page.goto("/logs");
    const select = page.locator("#log-process");
    await expect(select.locator("option")).toHaveCount(7);
    await expect(select.locator("option[value='gateway']"))
      .toHaveText("gateway");
  });

  test("level filter has expected options", async ({ page }) => {
    await page.goto("/logs");
    const select = page.locator("#log-level");
    await expect(select.locator("option[value='error']"))
      .toHaveText("error");
  });

  test("has error aggregation card", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.getByRole("heading", { name: "Error Aggregation" })).toBeVisible();
  });

  // New interactive tests (9 total)

  test("full line visibility: long lines are fully visible via scroll or wrap", async ({ page }) => {
    await page.goto("/logs");
    const logView = page.locator("#log-view");
    await waitForHTMX(page, 2000);

    // Log lines should be visible (wrapped or scrollable)
    const logLines = logView.locator("div");
    const count = await logLines.count();

    if (count > 0) {
      const firstLine = logLines.first();
      await expect(firstLine).toBeVisible();

      // Check if line has whitespace-pre-wrap or overflow
      const classes = await firstLine.getAttribute("class");
      expect(classes).toMatch(/whitespace-pre-wrap|break-all|overflow/);
    }
  });

  test("quick filters: click gateway chip applies instant filter", async ({ page }) => {
    await page.goto("/logs");
    const logView = page.locator("#log-view");
    await waitForHTMX(page, 2000);

    // Click gateway quick filter
    const gatewayBtn = page.locator("button", { hasText: /^gateway$/ });
    await gatewayBtn.click();
    await waitForHTMX(page);

    // Verify filter was applied
    const processSelect = page.locator("#log-process");
    const value = await processSelect.inputValue();
    expect(value).toBe("gateway");
  });

  test("smart search: type multiple keywords applies all filters", async ({ page }) => {
    await page.goto("/logs");
    const smartSearch = page.locator("#smart-search");

    // Type smart search query
    await smartSearch.fill("gateway error order");
    await smartSearch.press("Enter");
    await waitForHTMX(page);

    // Verify filters applied
    const processSelect = page.locator("#log-process");
    const levelSelect = page.locator("#log-level");
    const searchInput = page.locator("#log-search");

    const process = await processSelect.inputValue();
    const level = await levelSelect.inputValue();
    const search = await searchInput.inputValue();

    // Should extract "gateway" as process, "error" as level, "order" as search
    expect(process).toBe("gateway");
    expect(level).toBe("error");
    expect(search).toBe("order");
  });

  test("keyboard shortcuts: press / focuses search", async ({ page }) => {
    await page.goto("/logs");
    await waitForHTMX(page, 1000);

    // Press / key
    await page.keyboard.press("/");

    // Smart search should be focused
    const smartSearch = page.locator("#smart-search");
    await expect(smartSearch).toBeFocused();
  });

  test("filter clearing: press Ctrl+L clears all filters", async ({ page }) => {
    await page.goto("/logs");

    // Set some filters
    await page.locator("#log-process").selectOption("gateway");
    await page.locator("#log-level").selectOption("error");
    await page.locator("#log-search").fill("test");
    await waitForHTMX(page);

    // Press Ctrl+L
    await page.keyboard.press("Control+l");
    await waitForHTMX(page);

    // All filters should be cleared
    const processVal = await page.locator("#log-process").inputValue();
    const levelVal = await page.locator("#log-level").inputValue();
    const searchVal = await page.locator("#log-search").inputValue();

    expect(processVal).toBe("");
    expect(levelVal).toBe("");
    expect(searchVal).toBe("");
  });

  test("line expansion: click line shows full content in modal", async ({ page }) => {
    await page.goto("/logs");
    const logView = page.locator("#log-view");
    await waitForHTMX(page, 2000);

    const logLines = logView.locator("div[onclick*='showFullLine']");
    const count = await logLines.count();

    if (count > 0) {
      // Click first log line
      await logLines.first().click();

      // Modal should appear
      const modal = page.locator("#log-modal");
      await expect(modal).not.toHaveClass(/hidden/);

      // Modal content should be visible
      const modalContent = page.locator("#modal-content");
      await expect(modalContent).toBeVisible();
    }
  });

  test("copy functionality: click copy button copies full line", async ({ page }) => {
    await page.goto("/logs");
    const logView = page.locator("#log-view");
    await waitForHTMX(page, 2000);

    const logLines = logView.locator("div[onclick*='showFullLine']");
    const count = await logLines.count();

    if (count > 0) {
      // Click line to open modal
      await logLines.first().click();

      // Click copy button
      const copyBtn = page.locator("button", { hasText: "Copy" });
      await copyBtn.click();

      // Button should show "Copied!" temporarily
      await expect(copyBtn).toHaveText(/Copied!/);
    }
  });

  test("auto-refresh with filters: filters persist across auto-refresh", async ({ page }) => {
    await page.goto("/logs");

    // Set filter
    await page.locator("#log-process").selectOption("risk");
    await waitForHTMX(page);

    const initialFilter = await page.locator("#log-process").inputValue();
    expect(initialFilter).toBe("risk");

    // Wait for auto-refresh (2s interval)
    await waitForRefresh(2000);

    // Filter should still be set
    const afterFilter = await page.locator("#log-process").inputValue();
    expect(afterFilter).toBe("risk");
  });

  test("log viewer loads without console errors", async ({ page }) => {
    const errors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        errors.push(msg.text());
      }
    });

    await page.goto("/logs");
    await waitForHTMX(page, 2000);

    // Trigger some interactions
    await page.locator("button", { hasText: /^gateway$/ }).click();
    await waitForHTMX(page);

    await page.waitForTimeout(1000);
    expect(errors.length).toBe(0);
  });
});
