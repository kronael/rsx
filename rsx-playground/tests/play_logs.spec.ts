import { test, expect } from "@playwright/test";
import { waitForHTMX, waitForRefresh, pressSlashForSearch, pressClearShortcut } from "./test_helpers";

test.describe("Logs tab", () => {
  test("loads and has filters", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.locator("nav a", { hasText: "Logs" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Unified Log" })).toBeVisible();
    // Filter dropdowns are hidden (smart search sets them)
    await expect(page.locator("#log-process")).toBeAttached();
    await expect(page.locator("#log-level")).toBeAttached();
    await expect(page.locator("#log-search")).toBeAttached();
    // Smart search is visible
    await expect(page.locator("#smart-search")).toBeVisible();
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
    await page.waitForLoadState("networkidle");
    const logView = page.locator("#log-view");

    // Log lines should be visible (wrapped or scrollable)
    const logLines = logView.locator("div");
    const count = await logLines.count();

    if (count > 0) {
      const firstLine = logLines.first();
      await expect(firstLine).toBeVisible();

      // Check if line has whitespace-pre-wrap or overflow
      const classes = await firstLine.getAttribute("class") ?? "";
      const style = await firstLine.getAttribute("style") ?? "";
      expect(classes + style).toMatch(
        /whitespace|break|overflow|pre-wrap|scroll/
      );
    }
  });

  test("quick filters: click gateway chip applies instant filter", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    // Click gateway quick filter
    const gatewayBtn = page.locator("button", { hasText: /^gateway$/ });
    await gatewayBtn.click();
    await waitForHTMX(page);

    // Verify hidden filter was set via JavaScript
    const value = await page.evaluate(() => {
      const el = document.getElementById("log-process") as HTMLSelectElement;
      return el?.value;
    });
    expect(value).toBe("gateway");
  });

  test("smart search: type multiple keywords applies all filters", async ({ page }) => {
    await page.goto("/logs");
    const smartSearch = page.locator("#smart-search");

    // Type smart search query
    await smartSearch.fill("gateway error order");
    await smartSearch.press("Enter");
    await waitForHTMX(page);

    // Verify hidden filters were set via JavaScript
    const values = await page.evaluate(() => {
      return {
        process: (document.getElementById("log-process") as HTMLSelectElement)?.value,
        level: (document.getElementById("log-level") as HTMLSelectElement)?.value,
        search: (document.getElementById("log-search") as HTMLInputElement)?.value,
      };
    });

    // Should extract "gateway" as process, "error" as level, "order" as search
    expect(values.process).toBe("gateway");
    expect(values.level).toBe("error");
    expect(values.search).toBe("order");
  });

  test("keyboard shortcuts: press / focuses search", async ({ page }) => {
    await page.goto("/logs");
    // Wait for page to fully settle under load
    await page.waitForLoadState("networkidle");

    // Focus the page, then press /
    await page.locator("h2").first().click();
    await page.keyboard.press("/");

    // Smart search should be focused
    const smartSearch = page.locator("#smart-search");
    await expect(smartSearch).toBeFocused({ timeout: 5000 });
  });

  test("filter clearing: press Ctrl+L clears all filters", async ({ page }) => {
    await page.goto("/logs");

    // Set filters via smart search
    const smartSearch = page.locator("#smart-search");
    await smartSearch.fill("gateway error test");
    await smartSearch.press("Enter");
    await waitForHTMX(page);

    // Press Ctrl+L to clear
    await page.keyboard.press("Control+l");
    await waitForHTMX(page);

    // All hidden filters should be cleared
    const values = await page.evaluate(() => {
      return {
        process: (document.getElementById("log-process") as HTMLSelectElement)?.value,
        level: (document.getElementById("log-level") as HTMLSelectElement)?.value,
        search: (document.getElementById("log-search") as HTMLInputElement)?.value,
        smart: (document.getElementById("smart-search") as HTMLInputElement)?.value,
      };
    });

    expect(values.process).toBe("");
    expect(values.level).toBe("");
    expect(values.search).toBe("");
    expect(values.smart).toBe("");
  });

  test("line expansion: click line shows full content in modal", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");
    const logView = page.locator("#log-view");

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

  test("copy button exists in modal", async ({ page }) => {
    await page.goto("/logs");
    await page.waitForLoadState("networkidle");
    const logView = page.locator("#log-view");

    const logLines = logView.locator("div[onclick*='showFullLine']");
    const count = await logLines.count();

    if (count > 0) {
      // Click line to open modal
      await logLines.first().click();

      // Copy button should be visible in modal
      const copyBtn = page.locator("#log-modal button", { hasText: "Copy" });
      await expect(copyBtn).toBeVisible();
    }
  });

  test("auto-refresh with filters: filters persist across auto-refresh", async ({ page }) => {
    await page.goto("/logs");

    // Set filter via smart search
    const smartSearch = page.locator("#smart-search");
    await smartSearch.fill("risk");
    await smartSearch.press("Enter");
    await waitForHTMX(page);

    const initialFilter = await page.evaluate(() => {
      return (document.getElementById("log-process") as HTMLSelectElement)?.value;
    });
    expect(initialFilter).toBe("risk");

    // Wait for auto-refresh (2s interval)
    await waitForRefresh(2000);

    // Filter should still be set
    const afterFilter = await page.evaluate(() => {
      return (document.getElementById("log-process") as HTMLSelectElement)?.value;
    });
    expect(afterFilter).toBe("risk");
  });

  test("log viewer loads without console errors", async ({ page }) => {
    const errors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error"
        && !msg.text().includes("ERR_")
        && !msg.text().includes("htmx")
        && !msg.text().includes("Failed to fetch")) {
        errors.push(msg.text());
      }
    });

    await page.goto("/logs");
    await page.waitForLoadState("networkidle");

    // Trigger some interactions
    await page.locator("button", { hasText: /^gateway$/ }).click();
    await waitForHTMX(page);

    await page.waitForTimeout(1000);
    expect(errors.length).toBe(0);
  });
});
