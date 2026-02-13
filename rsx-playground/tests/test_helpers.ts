/**
 * Shared test helpers for RSX Playground E2E tests
 */

import { Page, expect } from "@playwright/test";

/**
 * Wait for HTMX to finish loading/swapping content.
 * Uses HTMX's built-in htmx:afterSwap event.
 */
export async function waitForHTMX(page: Page, timeout = 3000) {
  await page.waitForTimeout(100);
  await page.evaluate(() => {
    return new Promise((resolve) => {
      let timeoutId = setTimeout(() => resolve(true), 1000);
      document.body.addEventListener("htmx:afterSwap", () => {
        clearTimeout(timeoutId);
        resolve(true);
      }, { once: true });
    });
  });
}

/**
 * Wait for an auto-refresh interval plus buffer.
 * Used to verify HTMX auto-refresh behavior.
 */
export async function waitForRefresh(intervalMs: number, buffer = 500) {
  await new Promise((resolve) => setTimeout(resolve, intervalMs + buffer));
}

/**
 * Capture current state of an element, wait for refresh, verify it changed.
 */
export async function verifyAutoRefresh(
  page: Page,
  selector: string,
  intervalMs: number
) {
  const initialContent = await page.locator(selector).textContent();
  await waitForRefresh(intervalMs);
  const newContent = await page.locator(selector).textContent();
  // Content should have changed after refresh
  expect(newContent).not.toBe(initialContent);
}

/**
 * Verify HTMX swap by checking the target element updates after action.
 */
export async function verifyHTMXSwap(
  page: Page,
  triggerSelector: string,
  targetSelector: string
) {
  const initialContent = await page.locator(targetSelector).textContent();
  await page.locator(triggerSelector).click();
  await waitForHTMX(page);
  const newContent = await page.locator(targetSelector).textContent();
  expect(newContent).not.toBe(initialContent);
}

/**
 * Wait for a selector to contain specific text.
 */
export async function waitForText(
  page: Page,
  selector: string,
  text: string,
  timeout = 5000
) {
  await expect(page.locator(selector)).toContainText(text, { timeout });
}

/**
 * Verify table has at least minRows rows.
 */
export async function verifyTableHasRows(
  page: Page,
  selector: string,
  minRows = 1
) {
  const rows = page.locator(`${selector} tbody tr`);
  const count = await rows.count();
  expect(count).toBeGreaterThanOrEqual(minRows);
}

/**
 * Verify a metric/stat element contains a number.
 */
export async function verifyMetricHasValue(
  page: Page,
  selector: string
) {
  const content = await page.locator(selector).textContent();
  expect(content).toMatch(/\d+/); // Contains at least one digit
}

/**
 * Helper to check if element is visible and contains non-placeholder text.
 */
export async function verifyRealData(
  page: Page,
  selector: string,
  excludePatterns: string[] = ["loading", "no data", "--"]
) {
  await expect(page.locator(selector)).toBeVisible();
  const content = await page.locator(selector).textContent();
  for (const pattern of excludePatterns) {
    expect(content?.toLowerCase()).not.toContain(pattern);
  }
}

/**
 * Verify HTMX polling works by checking hx-trigger attribute.
 */
export async function verifyPolling(
  locator: any,
  expectedInterval: string
) {
  const trigger = await locator.getAttribute("hx-trigger");
  expect(trigger).toContain(expectedInterval);
}

/**
 * Helper to start RSX processes via the playground (for integration tests).
 * Returns true if successful.
 */
export async function startRSXProcesses(
  page: Page,
  scenario = "minimal"
): Promise<boolean> {
  await page.goto("/overview");
  await page.locator("#scenario").selectOption(scenario);
  await page.locator("button", { hasText: "Build & Start All" }).click();

  // Wait for build to complete
  await waitForText(page, "#start-result", "started", 120000);

  // Verify processes are running
  await waitForHTMX(page, 5000);
  const processTable = page.locator("table tbody tr");
  const count = await processTable.count();
  return count > 0;
}

/**
 * Stop all RSX processes.
 */
export async function stopRSXProcesses(page: Page) {
  await page.goto("/overview");
  await page.locator("button", { hasText: "Stop All" }).click();
  await waitForHTMX(page);
}

/**
 * Type into keyboard shortcut for log search.
 */
export async function pressSlashForSearch(page: Page) {
  await page.keyboard.press("/");
}

/**
 * Clear all filters with Ctrl+L.
 */
export async function pressClearShortcut(page: Page) {
  await page.keyboard.press("Control+l");
}
